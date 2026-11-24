//! End-to-end runs of the binary against the stand-in `op` in tests/support/bin.
//! PATH holds only that directory and the system's own, so a run can never
//! reach the real 1Password CLI.
#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
