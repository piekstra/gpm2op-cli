//! Removing the plaintext export once every row in it is in 1Password.
//!
//! Only the regular file the sync read is removed. A path that is a symlink
//! or anything other than a regular file is left alone, and so is a file that
//! changed while the sync ran, since it may hold rows this run did not read.

use std::fs::{self, FileType};
use std::io;
use std::path::Path;
use std::time::SystemTime;

/// What the export looked like when the sync read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    len: u64,
    modified: Option<SystemTime>,
}

impl Snapshot {
    /// Record the export at `path` before it is read. `None` when the path is
    /// not a regular file; such a path is never removed.
    pub fn of(path: &Path) -> Option<Snapshot> {
        let meta = fs::symlink_metadata(path).ok()?;
        meta.file_type().is_file().then(|| Snapshot {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

/// How a run ended, as far as removing the export is concerned.
#[derive(Debug, Clone, Copy)]
pub struct RunEnd {
    pub dry_run: bool,
    pub keep_csv: bool,
    pub credentials: usize,
    pub failed: usize,
}

/// Why the export was left on disk.
#[derive(Debug)]
pub enum Kept {
    DryRun,
    KeepFlag,
    NoCredentials,
    Failed(usize),
    NotRegularFile(&'static str),
    Changed,
    RemoveFailed(io::Error),
}

#[derive(Debug)]
pub enum Disposal {
    Deleted,
    Kept(Kept),
}

/// Whether a run that ended this way may remove the export at all.
pub fn may_delete(end: RunEnd) -> Result<(), Kept> {
    if end.dry_run {
        Err(Kept::DryRun)
    } else if end.keep_csv {
        Err(Kept::KeepFlag)
    } else if end.credentials == 0 {
        // Nothing was synced, and the file may not be a password export at all.
        Err(Kept::NoCredentials)
    } else if end.failed > 0 {
        Err(Kept::Failed(end.failed))
    } else {
        Ok(())
    }
}

/// Remove the export at `path` if the run allows it and the path still holds
/// the regular file that was read.
pub fn dispose(path: &Path, read: Option<&Snapshot>, end: RunEnd) -> Disposal {
    if let Err(kept) = may_delete(end) {
        return Disposal::Kept(kept);
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) => return Disposal::Kept(Kept::RemoveFailed(e)),
    };
    if !meta.file_type().is_file() {
        return Disposal::Kept(Kept::NotRegularFile(kind(&meta.file_type())));
    }
    let Some(read) = read else {
        return Disposal::Kept(Kept::Changed);
    };
    if Snapshot::of(path).as_ref() != Some(read) {
        return Disposal::Kept(Kept::Changed);
    }
    match fs::remove_file(path) {
        Ok(()) => Disposal::Deleted,
        Err(e) => Disposal::Kept(Kept::RemoveFailed(e)),
    }
}

fn kind(ft: &FileType) -> &'static str {
    if ft.is_symlink() {
        "a symlink"
    } else if ft.is_dir() {
        "a directory"
    } else {
        "a pipe, device or socket"
    }
}

/// The stderr note for an export left on disk, if it needs one.
pub fn kept_note(path: &str, kept: &Kept) -> Option<String> {
    Some(match kept {
        Kept::DryRun => return None,
        Kept::KeepFlag => format!(
            "Kept the export at {path} (--keep-csv). It holds your passwords in plain text; \
             delete it when you are done."
        ),
        Kept::NoCredentials => format!(
            "Left {path} in place: it holds no usable credentials, so nothing was synced from it."
        ),
        Kept::Failed(n) => format!(
            "Kept the export at {path}: {n} row(s) failed. Fix the cause and rerun; \
             a run with no failures deletes it."
        ),
        Kept::NotRegularFile(kind) => format!(
            "Left {path} in place: it is {kind}, not a regular file, so gpm2op does not delete it. \
             Delete the export it reads from yourself."
        ),
        Kept::Changed => format!(
            "Left {path} in place: it changed while the sync ran, so it may hold rows this run \
             did not read. Rerun to sync them; a run with no failures deletes it."
        ),
        Kept::RemoveFailed(e) => format!(
            "Could not delete the export at {path}: {e}. It holds your passwords in plain text; \
             delete it yourself."
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn clean() -> RunEnd {
        RunEnd {
            dry_run: false,
            keep_csv: false,
            credentials: 3,
            failed: 0,
        }
    }

    fn export_in(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("export.csv");
        fs::write(&path, "name,url,username,password,note\n").unwrap();
        path
    }

    #[test]
    fn a_clean_run_may_delete() {
        assert!(may_delete(clean()).is_ok());
    }

    #[test]
    fn dry_run_keep_flag_empty_and_failures_keep_the_file() {
        let cases = [
            (
                RunEnd {
                    dry_run: true,
                    ..clean()
                },
                "DryRun",
            ),
            (
                RunEnd {
                    keep_csv: true,
                    ..clean()
                },
                "KeepFlag",
            ),
            (
                RunEnd {
                    credentials: 0,
                    ..clean()
                },
                "NoCredentials",
            ),
            (
                RunEnd {
                    failed: 1,
                    ..clean()
                },
                "Failed",
            ),
        ];
        for (end, want) in cases {
            let got = format!("{:?}", may_delete(end).unwrap_err());
            assert!(got.starts_with(want), "{end:?}: got {got}");
        }
    }

    #[test]
    fn dispose_removes_the_file_it_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_in(dir.path());
        let read = Snapshot::of(&path);
        assert!(matches!(
            dispose(&path, read.as_ref(), clean()),
            Disposal::Deleted
        ));
        assert!(!path.exists());
    }

    #[test]
    fn dispose_keeps_a_file_that_changed_since_it_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_in(dir.path());
        let read = Snapshot::of(&path);
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "Late,https://late.example/,jane,pw,").unwrap();
        drop(f);
        let got = dispose(&path, read.as_ref(), clean());
        assert!(matches!(got, Disposal::Kept(Kept::Changed)), "{got:?}");
        assert!(path.exists());
    }

    #[test]
    fn dispose_leaves_a_directory_alone() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Snapshot::of(dir.path()).is_none());
        let got = dispose(dir.path(), None, clean());
        assert!(
            matches!(got, Disposal::Kept(Kept::NotRegularFile("a directory"))),
            "{got:?}"
        );
        assert!(dir.path().is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn dispose_leaves_a_symlink_and_its_target_alone() {
        let dir = tempfile::tempdir().unwrap();
        let target = export_in(dir.path());
        let link = dir.path().join("link.csv");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(Snapshot::of(&link).is_none());
        let got = dispose(&link, None, clean());
        assert!(
            matches!(got, Disposal::Kept(Kept::NotRegularFile("a symlink"))),
            "{got:?}"
        );
        assert!(link.symlink_metadata().is_ok());
        assert!(target.exists());
    }

    #[test]
    fn a_dry_run_needs_no_note_and_every_other_kept_file_gets_one() {
        assert!(kept_note("x.csv", &Kept::DryRun).is_none());
        for kept in [
            Kept::KeepFlag,
            Kept::NoCredentials,
            Kept::Failed(2),
            Kept::NotRegularFile("a symlink"),
            Kept::Changed,
            Kept::RemoveFailed(io::Error::from(io::ErrorKind::PermissionDenied)),
        ] {
            let note = kept_note("x.csv", &kept).unwrap();
            assert!(note.contains("x.csv"), "{note}");
        }
    }
}
