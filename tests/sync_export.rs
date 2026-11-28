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
        self.export_as(fixture, "Google Passwords.csv")
    }

    /// Copy a fixture export into the scratch directory as `name`.
    fn export_as(&self, fixture: &str, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::copy(repo("tests/fixtures").join(fixture), &path).unwrap();
        path
    }

    /// The templates the stand-in op was handed on `op item create`.
    fn created(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.path().join("created.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn tmp(&self) -> PathBuf {
        self.dir.path().join("tmp")
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.path().join("op.log")).unwrap_or_default()
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_of(Path::new(env!("CARGO_BIN_EXE_pw2op")), args)
    }

    fn command_of(&self, exe: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .env_remove("PW2OP_EDIT_TIMEOUT")
            .env_remove("GPM2OP_EDIT_TIMEOUT")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", repo("tests/support/bin").display()),
            )
            .env("FAKE_OP_DIR", repo("tests/fixtures/op"))
            .env("FAKE_OP_LOG", self.dir.path().join("op.log"))
            .env("FAKE_OP_CREATED", self.dir.path().join("created.jsonl"))
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
                .starts_with("pw2op-")
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
        err.contains("1 row(s) matched an existing login and were not compared. Their passwords are still in Google Password Manager. --update --dry-run lists the logins --update would compare; --update --force reconciles them"),
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
        serde_json::json!({"not_compared": 1, "host_only": 1, "title_only": 0, "deduped": 1, "unreadable": 0})
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
        stderr(&out).contains("1 row(s) had no URL and no username (row 3 of "),
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
    let export = t.export("google-passwords-export-no-usable-rows.csv");
    let out = t.run(&["sync", s(&export)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(export.exists(), "a file that synced nothing was deleted");
    assert!(
        stderr(&out).contains("no usable credentials"),
        "{}",
        stderr(&out)
    );
    assert!(t.log().is_empty(), "op was called:\n{}", t.log());
}

#[test]
fn a_file_that_is_not_a_password_export_is_refused_before_op_runs() {
    let t = Scratch::new();
    let export = t.export("not-a-password-export.csv");
    let out = t.run(&["sync", s(&export)]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(export.exists());
    assert!(
        stderr(&out).contains("not a password export pw2op reads"),
        "{}",
        stderr(&out)
    );
    assert!(!stderr(&out).contains("memo"), "the header was repeated");
    assert!(t.log().is_empty(), "op was called:\n{}", t.log());

    let doc = one_json_document(&t.run(&["--json", "sync", s(&export)]));
    assert_eq!(doc["error"]["code"], "usage");
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

    let export = t.export("google-passwords-export-no-usable-rows.csv");
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
    let out = t.run(&["sync", s(&export), "--update", "--force"]);
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
    let doc = one_json_document(&t.run(&["--json", "sync", s(&export), "--update", "--force"]));
    assert_eq!(doc["summary"]["updated"], 0);
    assert_eq!(doc["summary"]["unverified"]["host_only"], 1);
    assert_eq!(doc["items"][0]["result"], "exists");
    assert!(!t.log().contains("item edit"), "{}", t.log());
}

#[test]
fn update_hands_op_a_0600_template_and_removes_it() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export), "--update", "--force"]);
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
        .command(&["sync", s(&export), "--update", "--force"])
        .env("PW2OP_EDIT_TIMEOUT", "2m")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let warning = "warning: PW2OP_EDIT_TIMEOUT=\"2m\" is not a whole number of seconds above 0; using the default of 120 s.";
    assert_eq!(stderr(&out).matches(warning).count(), 1, "{}", stderr(&out));
}

#[test]
fn update_removes_its_template_when_op_edit_fails() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", s(&export), "--update", "--force"])
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
            .command(&["sync", s(&export), "--update", "--force"])
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
            panic!("pw2op still running after {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_sigterm_to_pw2op_alone_reaches_op_and_removes_the_template() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let mut child = t
        .command(&["sync", s(&export), "--update", "--force"])
        .env("FAKE_OP_EDIT_SLEEP", "30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // A supervisor's `kill <pid>`: the signal reaches pw2op only, while op
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
        .command(&["--json", "sync", s(&export), "--update", "--force"])
        .env("FAKE_OP_EDIT_SLEEP", "30")
        .env("PW2OP_EDIT_TIMEOUT", "1")
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
    let out = t.run(&["sync", s(&export), "--update", "--force"]);
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

#[test]
fn an_apple_passwords_export_is_detected_and_mapped() {
    let t = Scratch::new();
    let export = t.export_as("apple-passwords-export.csv", "Passwords.csv");
    let out = t.run(&["--json", "sync", s(&export)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc = one_json_document(&out);
    let summary = &doc["summary"];
    assert_eq!(summary["format"], "apple");
    assert_eq!(summary["sources"][0]["format"], "apple");
    assert_eq!(summary["mode"], "upsert");
    // example.com matches the vault's login; the other three are created,
    // the one with no password included.
    assert_eq!(summary["created"], 3);
    assert_eq!(summary["skipped_existing"], 1);
    assert_eq!(summary["otp"]["added"], 1);
    assert_eq!(
        summary["otp"]["ignored_rows"],
        serde_json::json!([{"file": 0, "row": 5}])
    );
    assert_eq!(summary["csv_deleted"], true);
    assert!(!export.exists());
    assert!(
        stderr(&out).contains("(Apple Passwords / Safari)"),
        "{}",
        stderr(&out)
    );
    let created = t.created();
    assert_eq!(created.len(), 3);
    let org = &created[0];
    assert_eq!(org["title"], "example.org (jane)");
    let field = |item: &Value, ty: &str| {
        item["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["type"] == ty)
            .cloned()
    };
    assert_eq!(
        field(org, "OTP").unwrap()["value"],
        "otpauth://totp/Example:jane?secret=FIXTUREFIXTURE&issuer=Example"
    );
    let notes = org["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["purpose"] == "NOTES")
        .unwrap();
    assert_eq!(notes["value"], "an apple note");
    let net = &created[1];
    assert_eq!(net["title"], "example.net (jane)");
    assert!(field(net, "CONCEALED").is_none(), "{net}");
    assert!(field(&created[2], "OTP").is_none());
    assert!(!t.log().contains("FIXTUREFIXTURE"), "{}", t.log());

    let export = t.export_as("apple-passwords-export.csv", "Passwords.csv");
    let out = t.run(&["sync", s(&export)]);
    let err = stderr(&out);
    assert!(
        err.contains("not an otpauth:// address (row 5 of "),
        "{err}"
    );
    assert!(err.contains("still in Apple Passwords"), "{err}");
    assert!(
        stdout(&out).contains("Apple Passwords still holds every password in it"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn the_human_summary_names_the_format() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export), "--dry-run"]);
    assert!(
        stderr(&out).contains("(Chrome / Google Password Manager)"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_row_with_no_password_is_created_and_never_blanks_a_stored_one() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export-empty-password.csv");
    let out = t
        .command(&["sync", s(&export)])
        .env("FAKE_OP_ITEM_LIST", "item-list-empty.json")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("Done: 1 created"), "{}", stdout(&out));

    // The same row against a vault that has the login: --update leaves it.
    let export = t.export("google-passwords-export-empty-password.csv");
    let out = t.run(&["sync", s(&export), "--update", "--force"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!t.log().contains("item edit"), "{}", t.log());
    assert!(!t.log().contains("item get"), "{}", t.log());
    assert!(stdout(&out).contains("1 left as-is"), "{}", stdout(&out));
    // Nothing to compare, so nothing is unverified, but the row's note did
    // not reach the existing login.
    assert!(!stderr(&out).contains("not compared"), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("1 row(s) had a note for a login that already exists"),
        "{}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains("is in 1Password now"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn keep_all_imports_every_distinct_password_across_exports_once() {
    let t = Scratch::new();
    let chrome = t.export("google-passwords-export-keep-all.csv");
    let apple = t.export_as("apple-passwords-export-keep-all.csv", "Passwords.csv");
    let out = t.run(&[
        "--json",
        "sync",
        s(&chrome),
        s(&apple),
        "--keep-all",
        "--tag",
        "pw2op-import",
        "--tag",
        "second",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc = one_json_document(&out);
    let summary = &doc["summary"];
    assert_eq!(summary["format"], "mixed");
    assert_eq!(summary["mode"], "keep-all");
    // example.com's old password is the vault's own; the other three are new.
    assert_eq!(summary["created"], 3, "{summary}");
    assert_eq!(summary["unchanged"], 1);
    assert_eq!(summary["repeats"], 2);
    assert_eq!(summary["conflicting_logins"], 2);
    assert_eq!(summary["unverified"]["deduped"], 0);
    assert_eq!(summary["unverified"]["not_compared"], 0);
    assert_eq!(
        summary["tags"],
        serde_json::json!(["pw2op-import", "second"])
    );
    assert_eq!(summary["csv_deleted"], true);
    assert!(!chrome.exists() && !apple.exists());

    let log = t.log();
    assert_eq!(log.matches("item get").count(), 1, "{log}");
    assert_eq!(
        log.matches("--tags pw2op-import,second").count(),
        3,
        "{log}"
    );
    for pw in [
        "fixture-password-1",
        "fixture-password-2",
        "fixture-password-8",
    ] {
        assert!(!log.contains(pw), "{pw} was on op's argv:\n{log}");
    }

    let created = t.created();
    let passwords: Vec<&str> = created
        .iter()
        .map(|item| {
            item["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["purpose"] == "PASSWORD")
                .unwrap()["value"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(
        passwords,
        vec![
            "fixture-password-1",
            "fixture-password-2",
            "fixture-password-8"
        ]
    );
    // The repeat's different note is kept with the row it repeated.
    let note = created[0]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["purpose"] == "NOTES")
        .unwrap();
    assert_eq!(note["value"], "chrome note\n\napple note");
}

#[test]
fn keep_all_names_conflicts_by_count_in_human_mode() {
    let t = Scratch::new();
    let chrome = t.export("google-passwords-export-keep-all.csv");
    let apple = t.export_as("apple-passwords-export-keep-all.csv", "Passwords.csv");
    let out = t.run(&["sync", s(&chrome), s(&apple), "--keep-all"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("Read 6 credential(s) from 2 exports"), "{err}");
    assert!(err.contains("keeping every password"), "{err}");
    assert!(
        err.contains("2 site and username pair(s) have more than one password"),
        "{err}"
    );
    assert!(err.contains("2 row(s) repeated an earlier row's"), "{err}");
    assert_eq!(stdout(&out).matches("Deleted the export at").count(), 2);
}

#[test]
fn a_failed_row_keeps_every_export_of_the_run() {
    let t = Scratch::new();
    let refused = t.export("google-passwords-export-with-refused-row.csv");
    let apple = t.export_as("apple-passwords-export.csv", "Passwords.csv");
    let out = t.run(&["sync", s(&refused), s(&apple)]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(refused.exists() && apple.exists());
}

#[test]
fn bad_arguments_are_refused_before_op_runs() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    for args in [
        vec!["sync", s(&export), "--keep-all", "--update", "--force"],
        vec!["sync", s(&export), "--tag", "a,b"],
        vec!["sync", s(&export), "--tag", " "],
        vec!["sync", s(&export), s(&export)],
    ] {
        let out = t.run(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        assert!(export.exists(), "{args:?}");
    }
    assert!(t.log().is_empty(), "op was called:\n{}", t.log());
}

#[test]
fn update_never_edits_a_login_that_holds_a_passkey() {
    // op's help: a JSON template edit overwrites an item's passkey. The
    // passkey field's shape in this fixture is an assumption; the guard
    // matches any key or field type naming a passkey.
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["--json", "sync", s(&export), "--update", "--force"])
        .env("FAKE_OP_ITEM_GET", "item-get-revealed-passkey.json")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let doc = one_json_document(&out);
    assert_eq!(doc["summary"]["failed"], 1);
    assert!(doc["items"][0]["result"]
        .as_str()
        .unwrap()
        .contains("holds a passkey"));
    assert!(!t.log().contains("item edit"), "{}", t.log());
    assert!(export.exists());
}

#[test]
fn update_needs_force_outside_a_dry_run() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t.run(&["sync", s(&export), "--update"]);
    assert_eq!(out.status.code(), Some(6), "{}", stderr(&out));
    assert!(stderr(&out).contains("passkey"), "{}", stderr(&out));
    assert!(t.log().is_empty(), "op was called:\n{}", t.log());
    assert!(export.exists());
    let doc = one_json_document(&t.run(&["--json", "sync", s(&export), "--update"]));
    assert_eq!(doc["error"]["code"], "confirmation_required");

    let out = t.run(&["sync", s(&export), "--update", "--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("would update"), "{}", stdout(&out));
}

#[test]
fn keep_all_fails_a_row_whose_comparison_read_fails_and_keeps_every_export() {
    for dry_run in [false, true] {
        let t = Scratch::new();
        let chrome = t.export("google-passwords-export-keep-all.csv");
        let apple = t.export_as("apple-passwords-export-keep-all.csv", "Passwords.csv");
        let mut args = vec!["--json", "sync", s(&chrome), s(&apple), "--keep-all"];
        if dry_run {
            args.push("--dry-run");
        }
        let out = t
            .command(&args)
            .env("FAKE_OP_ITEM_GET_FAIL", "1")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        let doc = one_json_document(&out);
        // Both example.com passwords need the vault login's password read.
        assert_eq!(doc["summary"]["failed"], 2, "{doc}");
        assert_eq!(doc["summary"]["unchanged"], 0);
        let created = t.created();
        assert!(
            created.iter().all(|i| i["title"] != "Example"),
            "an unread example.com row was created: {created:?}"
        );
        assert!(chrome.exists() && apple.exists(), "dry_run={dry_run}");
    }
}

#[test]
fn default_mode_keeps_an_earlier_rows_code_password_and_note() {
    // An Apple row with a code and a note, then a Chrome row for the same
    // site and username with neither, and no password.
    let t = Scratch::new();
    let apple = t.export_as("apple-passwords-export.csv", "Passwords.csv");
    let chrome = t.export("google-passwords-export-same-login-as-apple.csv");
    let out = t
        .command(&["--json", "sync", s(&apple), s(&chrome)])
        .env("FAKE_OP_ITEM_LIST", "item-list-empty.json")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let created = t.created();
    let org = created
        .iter()
        .find(|i| i["urls"][0]["href"] == "https://www.example.org/")
        .unwrap();
    let fields = org["fields"].as_array().unwrap();
    let value =
        |pred: &dyn Fn(&Value) -> bool| fields.iter().find(|f| pred(f)).map(|f| f["value"].clone());
    assert_eq!(
        value(&|f| f["purpose"] == "PASSWORD"),
        Some(Value::from("fixture-password-2"))
    );
    assert!(value(&|f| f["type"] == "OTP").is_some(), "{org}");
    assert_eq!(
        value(&|f| f["purpose"] == "NOTES"),
        Some(Value::from("an apple note"))
    );
    assert_eq!(org["title"], "Example Org");
}

#[test]
fn a_code_that_did_not_reach_1password_stops_the_complete_claim() {
    // The vault already holds example.org's password; under keep-all the row
    // is unchanged, so its code and note are not added.
    let t = Scratch::new();
    let apple = t.export_as("apple-passwords-export.csv", "Passwords.csv");
    let out = t
        .command(&["sync", s(&apple), "--keep-all"])
        .env("FAKE_OP_ITEM_LIST", "item-list-whole-export.json")
        .env("FAKE_OP_ITEM_GET", "item-get-revealed-example-org.json")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("one-time-password secret for a login that already exists"),
        "{err}"
    );
    assert!(
        err.contains("had a note for a login that already exists"),
        "{err}"
    );
    assert!(
        !stdout(&out).contains("is in 1Password now"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn a_row_with_no_url_and_no_title_is_not_created_twice() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export-untitled.csv");
    let out = t
        .command(&["--json", "sync", s(&export), "--update", "--force"])
        .env("FAKE_OP_ITEM_LIST", "item-list-untitled.json")
        .env("FAKE_OP_ITEM_GET", "item-get-revealed-untitled.json")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let doc = one_json_document(&out);
    assert_eq!(doc["summary"]["created"], 0, "{doc}");
    assert_eq!(doc["summary"]["unchanged"], 1);
    assert_eq!(doc["summary"]["updated"], 0);
    assert!(!t.log().contains("item edit"), "{}", t.log());

    // A different password for the same username is a new login, not an edit.
    let export = t.export("google-passwords-export-untitled.csv");
    let out = t
        .command(&["--json", "sync", s(&export), "--update", "--force"])
        .env("FAKE_OP_ITEM_LIST", "item-list-untitled.json")
        .output()
        .unwrap();
    let doc = one_json_document(&out);
    assert_eq!(doc["summary"]["created"], 1, "{doc}");
    assert!(!t.log().contains("item edit"), "{}", t.log());
}

/// A copy of the alias in a directory of its own, as a 0.4.0 install that
/// self-updated has it: no `pw2op` beside it.
fn lone_alias(t: &Scratch) -> std::path::PathBuf {
    let dir = t.dir.path().join("lone");
    std::fs::create_dir_all(&dir).unwrap();
    let alias = dir.join("gpm2op");
    std::fs::copy(env!("CARGO_BIN_EXE_gpm2op"), &alias).unwrap();
    alias
}

#[test]
fn the_gpm2op_alias_alone_runs_pw2op_under_its_old_name() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let alias = lone_alias(&t);
    let out = t
        .command_of(&alias, &["--json", "sync", s(&export), "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(one_json_document(&out)["schema"], "sync-result/v1");
    assert!(
        stderr(&out).contains("gpm2op is now pw2op"),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("Install pw2op beside"),
        "{}",
        stderr(&out)
    );

    let help = t.command_of(&alias, &["--help"]).output().unwrap();
    assert!(stdout(&help).contains("Usage: gpm2op"), "{}", stdout(&help));
    let help = t.run(&["--help"]);
    assert!(stdout(&help).contains("Usage: pw2op"), "{}", stdout(&help));

    // Completions register the name the user runs.
    let zsh = t
        .command_of(&alias, &["completions", "zsh"])
        .output()
        .unwrap();
    assert!(
        stdout(&zsh).starts_with("#compdef gpm2op"),
        "{}",
        stdout(&zsh)
    );
    let zsh = t.run(&["completions", "zsh"]);
    assert!(
        stdout(&zsh).starts_with("#compdef pw2op"),
        "{}",
        stdout(&zsh)
    );
}

#[test]
fn the_gpm2op_alias_beside_pw2op_runs_it() {
    let t = Scratch::new();
    let dir = t.dir.path().join("both");
    std::fs::create_dir_all(&dir).unwrap();
    let alias = dir.join("gpm2op");
    std::fs::copy(env!("CARGO_BIN_EXE_gpm2op"), &alias).unwrap();
    // A stand-in pw2op that records its arguments, so the test shows which
    // binary ran.
    let pw2op = dir.join("pw2op");
    std::fs::write(&pw2op, "#!/bin/sh\necho \"ran pw2op with: $*\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&pw2op, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out = t
        .command_of(&alias, &["self-update", "--check"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "ran pw2op with: self-update --check\n");
    assert!(
        stderr(&out).contains("Call pw2op instead"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn the_alias_beside_the_real_pw2op_still_answers_as_gpm2op() {
    let t = Scratch::new();
    let dir = t.dir.path().join("installed");
    std::fs::create_dir_all(&dir).unwrap();
    let alias = dir.join("gpm2op");
    std::fs::copy(env!("CARGO_BIN_EXE_gpm2op"), &alias).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_pw2op"), dir.join("pw2op")).unwrap();

    let help = t.command_of(&alias, &["--help"]).output().unwrap();
    assert!(stdout(&help).contains("Usage: gpm2op"), "{}", stdout(&help));
    let zsh = t
        .command_of(&alias, &["completions", "zsh"])
        .output()
        .unwrap();
    assert!(
        stdout(&zsh).starts_with("#compdef gpm2op"),
        "{}",
        stdout(&zsh)
    );
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command_of(&alias, &["--json", "sync", s(&export), "--dry-run"])
        .env("FAKE_OP_LOG_ALIAS_ENV", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(one_json_document(&out)["schema"], "sync-result/v1");
    // The variable the alias set is not passed on to op.
    let log = fs::read_to_string(t.dir.path().join("op.log")).unwrap();
    assert!(log.contains("invoked-as unset"), "{log}");
    assert!(!log.contains("invoked-as gpm2op"), "{log}");
}

#[test]
fn a_pw2op_that_is_a_copy_of_the_alias_does_not_loop() {
    let t = Scratch::new();
    let dir = t.dir.path().join("copied");
    std::fs::create_dir_all(&dir).unwrap();
    let alias = dir.join("gpm2op");
    std::fs::copy(env!("CARGO_BIN_EXE_gpm2op"), &alias).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_gpm2op"), dir.join("pw2op")).unwrap();
    let mut child = t
        .command_of(&alias, &["--help"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            child.kill().unwrap();
            panic!("the alias kept execing itself");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(stdout(&out).contains("Usage: gpm2op"), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("this pw2op is a copy of the gpm2op alias"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn the_new_edit_timeout_variable_wins_over_the_old_one() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", s(&export), "--update", "--force"])
        .env("PW2OP_EDIT_TIMEOUT", "new")
        .env("GPM2OP_EDIT_TIMEOUT", "old")
        .output()
        .unwrap();
    assert!(
        stderr(&out).contains("warning: PW2OP_EDIT_TIMEOUT=\"new\""),
        "{}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("GPM2OP_EDIT_TIMEOUT"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn the_pre_rename_edit_timeout_variable_still_counts() {
    let t = Scratch::new();
    let export = t.export("google-passwords-export.csv");
    let out = t
        .command(&["sync", s(&export), "--update", "--force"])
        .env("GPM2OP_EDIT_TIMEOUT", "2m")
        .output()
        .unwrap();
    assert!(
        stderr(&out).contains("warning: GPM2OP_EDIT_TIMEOUT=\"2m\""),
        "{}",
        stderr(&out)
    );
}
