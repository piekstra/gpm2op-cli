//! Thin wrapper around the 1Password `op` CLI.
//!
//! All item writes go through JSON templates on **stdin** (create) or a
//! `0600` temp file (edit), never through assignment arguments — op itself
//! warns that command arguments are visible to other processes, so secrets
//! must not be passed that way.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::cell::Cell;
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new("op");
        cmd.args(self.base_args());
        cmd.args(args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd
    }

    /// Run `op` with args and optional stdin, returning stdout on success.
    fn run(&self, args: &[String], stdin: Option<&str>) -> Result<String> {
        let mut cmd = self.command(args);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = spawn(&mut cmd)?;

        if let Some(data) = stdin {
            child
                .stdin
                .take()
                .expect("stdin piped")
                .write_all(data.as_bytes())?;
        }

        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Err(failure(&String::from_utf8_lossy(&out.stderr)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run `op` while `hold` defers the stop signals, without waiting forever.
    /// A held signal is forwarded to `op`, which gets `STOP_GRACE` to exit
    /// before it is killed; `op` is also killed once `limit` passes, so a
    /// prompt nobody answers cannot hang the run.
    fn run_held(&self, args: &[String], hold: &SignalHold, limit: Duration) -> Result<()> {
        let mut cmd = self.command(args);
        cmd.stdin(Stdio::null());
        hold.release_in_child(&mut cmd);
        let mut child = spawn(&mut cmd)?;
        // Readers keep op from blocking on a full pipe. Started under the
        // hold, they inherit its mask.
        let stdout = child.stdout.take().map(drain);
        let stderr = child.stderr.take().map(drain);
        let waited = match supervise(&mut child, hold, limit) {
            Ok(waited) => waited,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        };
        let stderr = stderr.and_then(collect).unwrap_or_default();
        drop(stdout.and_then(collect));
        match waited {
            Waited::Exited(status) if status.success() => Ok(()),
            Waited::Exited(_) => Err(failure(&String::from_utf8_lossy(&stderr))),
            Waited::TimedOut => Err(Error::Op(format!(
                "`op item edit` did not finish within {} s and was stopped; an unanswered \
                 1Password prompt is the usual cause, so rerun when it can be approved",
                limit.as_secs()
            ))),
        }
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
    /// when dropped, on every return and on a panic. `SignalHold` defers a stop
    /// signal until after that drop, and `run_held` forwards it to `op` and
    /// bounds the wait (`GPM2OP_EDIT_TIMEOUT`).
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
        let res = self.run_held(
            &[
                "item".into(),
                "edit".into(),
                id.into(),
                "--template".into(),
                path,
            ],
            &hold,
            edit_timeout(),
        );
        drop(tmp);
        drop(hold);
        res
    }
}

fn spawn(cmd: &mut Command) -> Result<Child> {
    cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::OpMissing
        } else {
            Error::Spawn(e.to_string())
        }
    })
}

/// The error for an `op` call that exited non-zero, from its stderr.
fn failure(stderr: &str) -> Error {
    let stderr = stderr.trim().to_string();
    let lc = stderr.to_lowercase();
    if lc.contains("not signed in")
        || lc.contains("no account")
        || lc.contains("you are not currently signed in")
    {
        return Error::OpNotSignedIn;
    }
    Error::Op(stderr)
}

const DEFAULT_EDIT_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);

/// How long `op item edit` may run: `GPM2OP_EDIT_TIMEOUT` seconds, else 120.
/// A value that is set but rejected gets one stderr warning per run.
fn edit_timeout() -> Duration {
    static TIMEOUT: OnceLock<Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let raw = std::env::var_os("GPM2OP_EDIT_TIMEOUT");
        let raw = raw.as_ref().map(|v| v.to_string_lossy());
        match parse_edit_timeout(raw.as_deref()) {
            Ok(limit) => limit,
            Err(rejected) => {
                eprintln!(
                    "warning: GPM2OP_EDIT_TIMEOUT={rejected:?} is not a whole number of seconds \
                     above 0; using the default of {} s.",
                    DEFAULT_EDIT_TIMEOUT.as_secs()
                );
                DEFAULT_EDIT_TIMEOUT
            }
        }
    })
}

/// The edit bound from `GPM2OP_EDIT_TIMEOUT`'s value: unset means the
/// default; a whole number of seconds above 0, surrounding whitespace
/// allowed, is taken; anything else comes back as `Err` with the value.
fn parse_edit_timeout(raw: Option<&str>) -> std::result::Result<Duration, String> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_EDIT_TIMEOUT);
    };
    match raw.trim().parse::<u64>() {
        Ok(secs) if secs > 0 => Ok(Duration::from_secs(secs)),
        _ => Err(raw.to_string()),
    }
}

fn drain<R: Read + Send + 'static>(mut pipe: R) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    })
}

/// A reader's bytes, unless something op started still holds the pipe open a
/// moment after op exited.
fn collect(reader: JoinHandle<Vec<u8>>) -> Option<Vec<u8>> {
    let until = Instant::now() + Duration::from_secs(1);
    while !reader.is_finished() && Instant::now() < until {
        std::thread::sleep(POLL);
    }
    reader.is_finished().then(|| reader.join().ok()).flatten()
}

enum Waited {
    Exited(ExitStatus),
    TimedOut,
}

/// Wait for `child`, forwarding the first held signal to it and killing it
/// once the grace after that signal, or `limit`, runs out.
fn supervise(child: &mut Child, hold: &SignalHold, limit: Duration) -> std::io::Result<Waited> {
    let started = Instant::now();
    let mut stop_by: Option<Instant> = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Waited::Exited(status));
        }
        if stop_by.is_none() && hold.forward_pending(child.id()) {
            stop_by = Some(Instant::now() + STOP_GRACE);
        }
        let now = Instant::now();
        let signalled = stop_by.is_some();
        if stop_by.is_some_and(|t| now >= t) || now.duration_since(started) >= limit {
            let _ = child.kill();
            let status = child.wait()?;
            return Ok(if signalled {
                Waited::Exited(status)
            } else {
                Waited::TimedOut
            });
        }
        std::thread::sleep(POLL);
    }
}

/// Defers SIGINT, SIGTERM, SIGHUP and SIGQUIT while it lives, so a stop
/// signal ends the process only after the revealed item's temp file is gone.
/// `supervise` forwards a held signal to `op`, since a signal sent to gpm2op
/// alone (`kill`, `timeout`, a supervisor) never reaches it. When the hold
/// drops, the pending signal is delivered, and re-raised if it was not.
///
/// The mask is per thread, so the hold assumes gpm2op is single-threaded
/// while it lives, apart from threads started under it, which inherit the
/// mask. Any other thread that leaves these signals unblocked would take a
/// process-directed one and end the process with the file still on disk.
/// A spawned child inherits the mask too (std does not reset it on macOS), so
/// `release_in_child` gives `op` the mask from before the hold.
struct SignalHold {
    #[cfg(unix)]
    held: libc::sigset_t,
    #[cfg(unix)]
    previous: Option<libc::sigset_t>,
    received: Cell<Option<i32>>,
}

impl SignalHold {
    #[cfg(unix)]
    fn new() -> SignalHold {
        // SAFETY: sigemptyset/sigaddset only write the local set, and
        // pthread_sigmask reads it and writes the previous mask into a local.
        unsafe {
            let mut held: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut held);
            for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
                libc::sigaddset(&mut held, sig);
            }
            let mut previous: libc::sigset_t = std::mem::zeroed();
            let ok = libc::pthread_sigmask(libc::SIG_BLOCK, &held, &mut previous) == 0;
            SignalHold {
                held,
                previous: ok.then_some(previous),
                received: Cell::new(None),
            }
        }
    }

    #[cfg(not(unix))]
    fn new() -> SignalHold {
        SignalHold {
            received: Cell::new(None),
        }
    }

    /// Start `cmd`'s process with the signal mask from before the hold, so
    /// `op` can be stopped by the signals it receives.
    #[cfg(unix)]
    fn release_in_child(&self, cmd: &mut Command) {
        use std::os::unix::process::CommandExt;
        if let Some(previous) = self.previous {
            // SAFETY: the closure runs in the forked child before exec and
            // calls only sigprocmask, which is async-signal-safe.
            unsafe {
                cmd.pre_exec(move || {
                    if libc::sigprocmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut()) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
    }

    #[cfg(not(unix))]
    fn release_in_child(&self, _cmd: &mut Command) {}

    /// If a held signal is pending, send it to `pid` and remember it. Peeking
    /// leaves it pending, so it is still delivered when the hold drops.
    #[cfg(unix)]
    fn forward_pending(&self, pid: u32) -> bool {
        if self.previous.is_none() {
            return false;
        }
        // SAFETY: sigpending writes a local set; sigismember reads it and the
        // held set; kill only sends a signal to the child we spawned and have
        // not yet reaped.
        unsafe {
            let mut pending: libc::sigset_t = std::mem::zeroed();
            if libc::sigpending(&mut pending) != 0 {
                return false;
            }
            for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
                if libc::sigismember(&self.held, sig) == 1 && libc::sigismember(&pending, sig) == 1
                {
                    self.received.set(Some(sig));
                    libc::kill(pid as libc::pid_t, sig);
                    return true;
                }
            }
        }
        false
    }

    #[cfg(not(unix))]
    fn forward_pending(&self, _pid: u32) -> bool {
        false
    }
}

#[cfg(unix)]
impl Drop for SignalHold {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            // SAFETY: restores the mask saved by `new` on this thread, then
            // re-raises a received signal if unblocking did not deliver it.
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
                if let Some(sig) = self.received.get() {
                    libc::raise(sig);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_timeout_takes_whole_seconds_above_zero_and_rejects_the_rest() {
        let cases: [(Option<&str>, std::result::Result<u64, &str>); 7] = [
            (None, Ok(120)),
            (Some("1"), Ok(1)),
            (Some(" 5 "), Ok(5)),
            (Some("0"), Err("0")),
            (Some("abc"), Err("abc")),
            (Some("-1"), Err("-1")),
            (Some(""), Err("")),
        ];
        for (raw, want) in cases {
            let got = parse_edit_timeout(raw).map(|d| d.as_secs());
            assert_eq!(got, want.map_err(str::to_string), "{raw:?}");
        }
    }
}
