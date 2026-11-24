//! Thin wrapper around the 1Password `op` CLI.
//!
//! All item writes go through JSON templates on **stdin** (create) or a
//! `0600` temp file (edit), never through assignment arguments — op itself
//! warns that command arguments are visible to other processes, so secrets
//! must not be passed that way.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};

/// An item as returned by `op item list`/`op item get` (the subset we use).
#[derive(Debug, Clone, Deserialize)]
pub struct OpItem {
    pub id: String,
    /// For logins this is typically the username (from `op item list`).
    #[serde(default)]
    pub additional_information: Option<String>,
    #[serde(default)]
    pub urls: Vec<OpUrl>,
    /// Populated by `op item get` (not by `item list`).
    #[serde(default)]
    pub fields: Vec<OpField>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpUrl {
    #[serde(default)]
    pub href: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpField {
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
}

impl OpItem {
    /// The username for a login, preferring an explicit USERNAME field, then
    /// the list-view `additional_information`.
    pub fn username(&self) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.purpose.as_deref() == Some("USERNAME"))
            .and_then(|f| f.value.as_deref())
            .or(self.additional_information.as_deref())
            .filter(|s| !s.is_empty())
    }
}

/// Handle to the `op` CLI bound to an optional account.
pub struct Op {
    account: Option<String>,
}

impl Op {
    pub fn new(account: Option<String>) -> Self {
        Op { account }
    }

    fn base_args(&self) -> Vec<String> {
        let mut a = Vec::new();
        if let Some(acct) = &self.account {
            a.push("--account".to_string());
            a.push(acct.clone());
        }
        a
    }

    /// Run `op` with args and optional stdin, returning stdout on success.
    fn run(&self, args: &[String], stdin: Option<&str>) -> Result<String> {
        let mut cmd = Command::new("op");
        cmd.args(self.base_args());
        cmd.args(args);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::OpMissing
            } else {
                Error::Spawn(e.to_string())
            }
        })?;

        if let Some(data) = stdin {
            child
                .stdin
                .take()
                .expect("stdin piped")
                .write_all(data.as_bytes())?;
        }

        let out = child.wait_with_output()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let lc = stderr.to_lowercase();
            if lc.contains("not signed in")
                || lc.contains("no account")
                || lc.contains("you are not currently signed in")
            {
                return Err(Error::OpNotSignedIn);
            }
            return Err(Error::Op(stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Verify `op` is installed and a session is available.
    ///
    /// Probes with `op vault list`, a real read: it succeeds whenever `op`
    /// can actually serve requests (desktop-app integration or a session
    /// token). `op whoami` is deliberately not used — it reports "not signed
    /// in" under the desktop-app integration even while reads work.
    pub fn preflight(&self) -> Result<String> {
        let out = self.run(
            &[
                "vault".into(),
                "list".into(),
                "--format".into(),
                "json".into(),
            ],
            None,
        )?;
        Ok(out.trim().to_string())
    }

    /// List login items in a vault.
    pub fn list_logins(&self, vault: &str) -> Result<Vec<OpItem>> {
        let out = self.run(
            &[
                "item".into(),
                "list".into(),
                "--categories".into(),
                "Login".into(),
                "--vault".into(),
                vault.into(),
                "--format".into(),
                "json".into(),
            ],
            None,
        )?;
        if out.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&out).map_err(|e| Error::Parse(format!("op item list: {e}")))
    }

    /// Fetch a single item with concealed fields revealed, as raw JSON. Used on
    /// the update path so we can patch only the password and write the rest of
    /// the item back unchanged.
    pub fn get_revealed_value(&self, id: &str) -> Result<serde_json::Value> {
        let out = self.run(
            &[
                "item".into(),
                "get".into(),
                id.into(),
                "--reveal".into(),
                "--format".into(),
                "json".into(),
            ],
            None,
        )?;
        serde_json::from_str(&out).map_err(|e| Error::Parse(format!("op item get: {e}")))
    }

    /// Create a login item from a JSON template piped over stdin.
    pub fn create_login(&self, vault: &str, template_json: &str) -> Result<String> {
        let out = self.run(
            &[
                "item".into(),
                "create".into(),
                "--vault".into(),
                vault.into(),
                "--format".into(),
                "json".into(),
                "-".into(),
            ],
            Some(template_json),
        )?;
        let created: OpItem =
            serde_json::from_str(&out).map_err(|e| Error::Parse(format!("op item create: {e}")))?;
        Ok(created.id)
    }

    /// Replace an item from a full JSON template written to a `0600` temp file.
    /// The file is removed as soon as `op` returns: `NamedTempFile` deletes it
    /// when dropped, on every return and on a panic, and `SignalHold` keeps a
    /// Ctrl-C from ending the process before that drop runs.
    pub fn edit_from_template(&self, id: &str, template_json: &str) -> Result<()> {
        let hold = SignalHold::new();
        let mut tmp = tempfile::Builder::new()
            .prefix("gpm2op-")
            .suffix(".json")
            .tempfile()?;
        // tempfile creates the file with 0600 on Unix by default.
        tmp.write_all(template_json.as_bytes())?;
        tmp.flush()?;
        let path = tmp.path().to_string_lossy().into_owned();
        let res = self.run(
            &[
                "item".into(),
                "edit".into(),
                id.into(),
                "--template".into(),
                path,
            ],
            None,
        );
        drop(tmp);
        drop(hold);
        res.map(|_| ())
    }
}

/// Defers SIGINT, SIGTERM, SIGHUP and SIGQUIT while it lives. A signal that
/// arrives meanwhile is delivered when the hold drops, so the process still
/// ends, but only after the revealed item's temp file is gone. `op` still gets
/// the signal: std clears the signal mask of every child it spawns.
struct SignalHold {
    #[cfg(unix)]
    previous: Option<libc::sigset_t>,
}

impl SignalHold {
    #[cfg(unix)]
    fn new() -> SignalHold {
        // SAFETY: sigemptyset/sigaddset only write the local set, and
        // pthread_sigmask reads it and writes the previous mask into a local.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
                libc::sigaddset(&mut set, sig);
            }
            let mut previous: libc::sigset_t = std::mem::zeroed();
            let ok = libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut previous) == 0;
            SignalHold {
                previous: ok.then_some(previous),
            }
        }
    }

    #[cfg(not(unix))]
    fn new() -> SignalHold {
        SignalHold {}
    }
}

#[cfg(unix)]
impl Drop for SignalHold {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            // SAFETY: restores the mask saved by `new` on this thread.
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
            }
        }
    }
}
