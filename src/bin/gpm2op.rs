//! `gpm2op`, the name pw2op had through v0.4.0, kept through v0.6.x.
//!
//! With `pw2op` installed beside it, the alias execs that binary, which then
//! answers as `gpm2op`, so both names always run the same release:
//! `self-update` through either updates `pw2op`. On its own (a 0.4.0 install
//! that self-updated, whose updater extracts only `gpm2op`) it runs the
//! bundled copy of the code.

use std::process::ExitCode;

/// Shared with `src/main.rs`: the name the user typed, and the mark that a
/// run was already handed on once.
const ALIAS_ENV: &str = "PW2OP_INVOKED_AS";

const MOVE_NOTE: &str =
    "note: gpm2op is now pw2op. Call pw2op instead: gpm2op is removed after v0.6.x.";
const COPY_NOTE: &str = "note: this pw2op is a copy of the gpm2op alias. Run `pw2op self-update` \
to replace it with the real pw2op from the latest release.";
const INSTALL_NOTE: &str = "note: gpm2op is now pw2op. Install pw2op beside this command \
(https://github.com/piekstra/pw2op-cli#renamed-from-gpm2op): gpm2op is removed after v0.6.x, \
and an install with only gpm2op cannot update itself after that.";

fn main() -> ExitCode {
    // A run already handed on (a `pw2op` that is a copy of or link to this
    // alias) runs here instead of execing again.
    let handed_on = std::env::var_os(ALIAS_ENV).is_some();
    std::env::remove_var(ALIAS_ENV);
    #[cfg(unix)]
    if !handed_on {
        if let Some(pw2op) = sibling() {
            use std::os::unix::process::CommandExt;
            eprintln!("{MOVE_NOTE}");
            let err = std::process::Command::new(&pw2op)
                .args(std::env::args_os().skip(1))
                .env(ALIAS_ENV, "gpm2op")
                .exec();
            eprintln!(
                "warning: could not run {}: {err}; running gpm2op's own copy.",
                pw2op.display()
            );
            return pw2op::main("gpm2op", "gpm2op");
        }
    }
    // A copy of the alias saved as `pw2op`: its self-update installs the
    // real `pw2op` into that file.
    if own_name().as_deref() == Some("pw2op") {
        eprintln!("{COPY_NOTE}");
        return pw2op::main("gpm2op", "pw2op");
    }
    eprintln!("{}", if handed_on { MOVE_NOTE } else { INSTALL_NOTE });
    pw2op::main("gpm2op", "gpm2op")
}

/// The running executable's file name.
fn own_name() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.file_name()?.to_string_lossy().into_owned())
}

/// The executable `pw2op` in this binary's own directory, if there is one.
fn sibling() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join("pw2op");
    let meta = std::fs::metadata(&path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    meta.is_file().then_some(path)
}
