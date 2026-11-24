//! End-to-end runs of the binary against the stand-in `op` in tests/support/bin.
//! PATH holds only that directory and the system's own, so a run can never
//! reach the real 1Password CLI.
#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// A scratch directory for one test: a copy of an export, the stand-in op's
/// call log, and the TMPDIR the binary writes its temp files to.
struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("tmp")).unwrap();
        Scratch { dir }
    }

    /// Copy a fixture export into the scratch directory under the name
    /// Google gives it.
    fn export(&self, fixture: &str) -> PathBuf {
        let path = self.dir.path().join("Google Passwords.csv");
        fs::copy(repo("tests/fixtures").join(fixture), &path).unwrap();
        path
    }

    fn tmp(&self) -> PathBuf {
        self.dir.path().join("tmp")
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.path().join("op.log")).unwrap_or_default()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_gpm2op"));
        cmd.args(args)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", repo("tests/support/bin").display()),
            )
            .env("FAKE_OP_DIR", repo("tests/fixtures/op"))
            .env("FAKE_OP_LOG", self.dir.path().join("op.log"))
            .env("TMPDIR", self.tmp())
            .stdin(Stdio::null());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Parse stdout as a stream of JSON values and require exactly one.
fn one_json_document(out: &Output) -> Value {
    let text = stdout(out);
    let docs: Vec<Value> = serde_json::Deserializer::from_str(&text)
        .into_iter::<Value>()
        .collect::<Result<_, _>>()
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{text}"));
    assert_eq!(docs.len(), 1, "want one JSON document on stdout:\n{text}");
    docs.into_iter().next().unwrap()
}

/// Paths of the --update templates the stand-in op was handed, with their modes.
fn templates(log: &str) -> Vec<(String, PathBuf)> {
    log.lines()
        .filter_map(|l| l.strip_prefix("template "))
        .map(|rest| {
            let (mode, path) = rest.split_once(' ').unwrap();
            (mode.to_string(), PathBuf::from(path))
        })
        .collect()
}

fn leftover_temp_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("gpm2op-")
        })
        .collect()
}

#[test]
fn a_clean_sync_deletes_the_export() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!export.exists(), "the export is still on disk");
    assert!(stdout(&out).contains("Done: 2 created, 0 updated, 0 unchanged, 1 left as-is"));
    assert!(
        stdout(&out).contains("Deleted the export at"),
        "{}",
        stdout(&out)
    );
    assert!(!stderr(&out).contains("Reminder"));
}

#[test]
fn a_run_that_checked_every_row_says_so() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", s(&export)])
        .env("FAKE_OP_ITEM_LIST", "item-list-empty.json")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("every credential read from it is in 1Password now"),
        "{}",
        stdout(&out)
    );
    assert!(!stderr(&out).contains("row(s)"), "{}", stderr(&out));
}

#[test]
fn rows_not_checked_against_1password_are_counted_and_named() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export-unverified-rows.csv");
    let out = t.run(&["sync", s(&export)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!export.exists(), "the export is still on disk");
    let err = stderr(&out);
    assert!(
        err.contains("1 row(s) matched an existing login and were not compared. Their passwords are still in Google Password Manager; run with --update to reconcile."),
        "{err}"
    );
    assert!(err.contains("1 row(s) had no username"), "{err}");
    assert!(err.contains("1 row(s) repeated a later row's"), "{err}");
    let out_text = stdout(&out);
    assert!(!out_text.contains("is in 1Password now"), "{out_text}");
    assert!(
        out_text.contains("Google Password Manager still holds every password in it"),
        "{out_text}"
    );

    let export = t.export("google-passwords-export-unverified-rows.csv");
    let doc = one_json_document(&t.run(&["--json", "sync", s(&export)]));
    assert_eq!(
        doc["summary"]["unverified"],
        serde_json::json!({"not_compared": 1, "host_only": 1, "deduped": 1, "unreadable": 0})
    );
    assert_eq!(doc["summary"]["created"], 1);
    assert_eq!(doc["summary"]["csv_deleted"], true);
}

#[test]
fn a_row_with_no_url_and_no_username_is_counted_as_unreadable() {
    // Every other row is created, so only the dropped row keeps the
    // deletion line from claiming 1Password holds everything.
    let t = Scratch::new();
    let export = t.export("google-passwords-export-unreadable-row.csv");
    let out = t
        .command(&["sync", s(&export)])
        .env("FAKE_OP_ITEM_LIST", "item-list-empty.json")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("Done: 1 created"), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("1 row(s) had no URL and no username, so they were not synced"),
        "{}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains("is in 1Password now"),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains("Google Password Manager still holds every password in it"),
        "{}",
        stdout(&out)
    );
    assert!(!export.exists());

    let export = t.export("google-passwords-export-unreadable-row.csv");
    let doc = one_json_document(
        &t.command(&["--json", "sync", s(&export)])
            .env("FAKE_OP_ITEM_LIST", "item-list-empty.json")
            .output()
            .unwrap(),
    );
    assert_eq!(doc["summary"]["unverified"]["unreadable"], 1);
    assert_eq!(doc["summary"]["created"], 1);
}

#[test]
fn a_sync_that_creates_nothing_still_deletes_the_export() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", s(&export)])
        .env("FAKE_OP_ITEM_LIST", "item-list-whole-export.json")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("Done: 0 created"), "{}", stdout(&out));
    assert!(!export.exists(), "the export is still on disk");
}

#[test]
fn keep_csv_keeps_the_export() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export), "--keep-csv"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(export.exists());
    assert!(stderr(&out).contains("(--keep-csv)"), "{}", stderr(&out));
    assert!(!stdout(&out).contains("Deleted"));
}

#[test]
fn a_failed_row_keeps_the_export_for_the_rerun() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export-with-refused-row.csv");
    let out = t.run(&["sync", s(&export)]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stdout(&out).contains("1 failed"), "{}", stdout(&out));
    assert!(export.exists(), "an export with a failed row was deleted");
    assert!(
        stderr(&out).contains("1 row(s) failed. Fix the cause and rerun"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_failed_row_reports_ops_message() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export-with-refused-row.csv");
    let doc = one_json_document(&t.run(&["--json", "sync", s(&export)]));
    let refused = doc["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["host"] == "fail.example")
        .unwrap();
    assert_eq!(
        refused["result"],
        "failed: [ERROR] 2026/01/01 00:00:00 the test refuses this item"
    );
}

#[test]
fn a_dry_run_keeps_the_export() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export), "--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(export.exists());
    assert!(
        !t.log().contains("item create"),
        "a dry run wrote:\n{}",
        t.log()
    );
}

#[test]
fn a_symlink_and_its_target_are_not_deleted() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let link = t.dir.path().join("link.csv");
    std::os::unix::fs::symlink(&export, &link).unwrap();
    let out = t.run(&["sync", s(&link)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(link.symlink_metadata().is_ok(), "the symlink was removed");
    assert!(export.exists(), "the symlink's target was removed");
    assert!(stderr(&out).contains("it is a symlink"), "{}", stderr(&out));
}

#[test]
fn an_export_read_from_stdin_is_not_deleted() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", "/dev/stdin"])
        .stdin(fs::File::open(&export).unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("2 created"), "{}", stdout(&out));
    assert!(export.exists());
    assert!(
        stderr(&out).contains("Left /dev/stdin in place"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_file_with_no_usable_rows_is_left_alone() {
    let t = Scratch::new();
    let export = t.export("not-a-password-export.csv");
    let out = t.run(&["sync", s(&export)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(export.exists(), "a file that synced nothing was deleted");
    assert!(
        stderr(&out).contains("no usable credentials"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn json_stdout_is_exactly_one_document() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");

    let dry = one_json_document(&t.run(&["--json", "sync", s(&export), "--dry-run"]));
    assert_eq!(dry["schema"], "sync-result/v1");
    assert_eq!(dry["summary"]["planned"], 3);
    assert_eq!(dry["summary"]["csv_deleted"], false);
    assert!(export.exists());

    // The subcommand's own --json flag behaves the same as the global one.
    let out = t.run(&["sync", s(&export), "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let done = one_json_document(&out);
    assert_eq!(done["summary"]["created"], 2);
    assert_eq!(done["summary"]["csv_deleted"], true);
    assert!(stderr(&out).contains("Read 3 credential(s)"));
    assert!(!export.exists());

    let export = t.export("google-passwords-export-with-refused-row.csv");
    let failed = one_json_document(&t.run(&["--json", "sync", s(&export)]));
    assert_eq!(failed["summary"]["failed"], 1);
    assert_eq!(failed["summary"]["csv_deleted"], false);

    let export = t.export("not-a-password-export.csv");
    let empty = one_json_document(&t.run(&["--json", "sync", s(&export)]));
    assert_eq!(empty["summary"]["csv_deleted"], false);
    assert_eq!(empty["items"], serde_json::json!([]));

    let check = one_json_document(&t.run(&["--json", "check"]));
    assert_eq!(check["schema"], "check-result/v1");
    assert_eq!(check["vault"], "Private");
    assert_eq!(check["logins"], 1);

    let missing = t.dir.path().join("missing.csv");
    let error = one_json_document(&t.run(&["--json", "sync", s(&missing)]));
    assert_eq!(error["error"]["code"], "usage");
}

#[test]
fn update_never_overwrites_a_login_matched_on_host_alone() {
    // The export's row has no username; 1Password holds one login on the
    // same host, for jane@example.com, which may be another account.
    let t = Scratch::new();
    let export = t.export("google-passwords-export-empty-username.csv");
    let out = t.run(&["sync", s(&export), "--update"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let log = t.log();
    assert!(!log.contains("item edit"), "an edit was issued:\n{log}");
    assert!(!log.contains("item get"), "the login was read:\n{log}");
    assert!(templates(&log).is_empty(), "{log}");
    assert!(stdout(&out).contains("0 updated"), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("on host alone, which may be another account, so that login was not updated, even with --update"),
        "{}",
        stderr(&out)
    );

    let export = t.export("google-passwords-export-empty-username.csv");
    let doc = one_json_document(&t.run(&["--json", "sync", s(&export), "--update"]));
    assert_eq!(doc["summary"]["updated"], 0);
    assert_eq!(doc["summary"]["unverified"]["host_only"], 1);
    assert_eq!(doc["items"][0]["result"], "exists");
    assert!(!t.log().contains("item edit"), "{}", t.log());
}

#[test]
fn update_hands_op_a_0600_template_and_removes_it() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export), "--update"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("1 updated"), "{}", stdout(&out));
    let templates = templates(&t.log());
    assert_eq!(templates.len(), 1, "{}", t.log());
    let (mode, path) = &templates[0];
    assert_eq!(mode, "600");
    assert!(
        !path.exists(),
        "the template is still at {}",
        path.display()
    );
    assert_eq!(leftover_temp_files(&t.tmp()), Vec::<PathBuf>::new());
}

#[test]
fn a_rejected_edit_timeout_warns_once_and_uses_the_default() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", s(&export), "--update"])
        .env("GPM2OP_EDIT_TIMEOUT", "2m")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let warning = "warning: GPM2OP_EDIT_TIMEOUT=\"2m\" is not a whole number of seconds above 0; using the default of 120 s.";
    assert_eq!(stderr(&out).matches(warning).count(), 1, "{}", stderr(&out));
}

#[test]
fn update_removes_its_template_when_op_edit_fails() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", s(&export), "--update"])
        .env("FAKE_OP_EDIT_FAIL", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(export.exists(), "an export with a failed row was deleted");
    let templates = templates(&t.log());
    assert_eq!(templates.len(), 1, "{}", t.log());
    assert!(
        !templates[0].1.exists(),
        "the template survived a failed edit"
    );
    assert_eq!(leftover_temp_files(&t.tmp()), Vec::<PathBuf>::new());
}

#[test]
fn update_removes_its_template_when_interrupted_mid_edit() {
    for signal in ["INT", "TERM"] {
        let t = Scratch::new();
        let export = t.export("google-passwords-export.csv");
        let out = t
            .command(&["sync", s(&export), "--update"])
            .env("FAKE_OP_EDIT_SIGNAL", signal)
            .output()
            .unwrap();
        let want = if signal == "INT" {
            libc::SIGINT
        } else {
            libc::SIGTERM
        };
        assert_eq!(
            out.status.signal(),
            Some(want),
            "{signal}: {:?}",
            out.status
        );
        let templates = templates(&t.log());
        assert_eq!(templates.len(), 1, "{}", t.log());
        assert!(
            !templates[0].1.exists(),
            "SIG{signal} left the template at {}",
            templates[0].1.display()
        );
        assert_eq!(leftover_temp_files(&t.tmp()), Vec::<PathBuf>::new());
        assert!(export.exists(), "an interrupted sync deleted the export");
    }
}

/// Wait for `child` up to `limit`; kill it and panic if it outlives that.
fn wait_within(child: &mut std::process::Child, limit: Duration) -> std::process::ExitStatus {
    let until = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            panic!("gpm2op still running after {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_sigterm_to_gpm2op_alone_reaches_op_and_removes_the_template() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let mut child = t
        .command(&["sync", s(&export), "--update"])
        .env("FAKE_OP_EDIT_SLEEP", "30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // A supervisor's `kill <pid>`: the signal reaches gpm2op only, while op
    // waits on a prompt nobody answers.
    let until = Instant::now() + Duration::from_secs(10);
    while templates(&t.log()).is_empty() {
        assert!(Instant::now() < until, "op item edit never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    let signalled = Instant::now();
    // SAFETY: kill only sends a signal. The pid is this test's own child,
    // still running and not yet reaped (wait_within reaps it below), so it
    // cannot name another process.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let status = wait_within(&mut child, Duration::from_secs(10));
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{status:?}");
    // op ends on the forwarded signal itself, well inside the 5 s grace
    // after which it would be killed.
    assert!(
        signalled.elapsed() < Duration::from_secs(3),
        "op did not stop on the forwarded SIGTERM: {:?}",
        signalled.elapsed()
    );
    let templates = templates(&t.log());
    assert!(!templates[0].1.exists(), "SIGTERM left the template");
    assert_eq!(leftover_temp_files(&t.tmp()), Vec::<PathBuf>::new());
    assert!(export.exists(), "an interrupted sync deleted the export");
}

#[test]
fn an_edit_that_never_finishes_is_stopped_and_fails_its_row() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let started = Instant::now();
    let out = t
        .command(&["--json", "sync", s(&export), "--update"])
        .env("FAKE_OP_EDIT_SLEEP", "30")
        .env("GPM2OP_EDIT_TIMEOUT", "1")
        .output()
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let doc = one_json_document(&out);
    assert_eq!(doc["summary"]["failed"], 1);
    assert_eq!(doc["summary"]["csv_deleted"], false);
    let stuck = doc["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["host"] == "example.com")
        .unwrap();
    assert!(
        stuck["result"]
            .as_str()
            .unwrap()
            .starts_with("failed: `op item edit` did not finish within 1 s and was stopped"),
        "{stuck}"
    );
    let templates = templates(&t.log());
    assert!(!templates[0].1.exists(), "a stopped edit left the template");
    assert_eq!(leftover_temp_files(&t.tmp()), Vec::<PathBuf>::new());
    assert!(export.exists());
}

#[test]
fn no_password_reaches_op_argv() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export), "--update"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let log = t.log();
    assert!(log.contains("item create"), "{log}");
    for pw in [
        "fixture-password-1",
        "fixture-password-2",
        "fixture-password-3",
    ] {
        assert!(!log.contains(pw), "{pw} was on op's argv:\n{log}");
    }
}
