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

const COPY_NOTE: &str = "note: this pw2op is the gpm2op alias, not pw2op. Replace it with pw2op \
from the release archive, or run `cargo install --git https://github.com/piekstra/pw2op-cli --force` \
(https://github.com/piekstra/pw2op-cli#renamed-from-gpm2op).";
const INSTALL_NOTE: &str = "note: gpm2op is now pw2op. Install pw2op beside this command \
(https://github.com/piekstra/pw2op-cli#renamed-from-gpm2op): gpm2op is removed after v0.6.x, \
and an install with only gpm2op cannot update itself after that.";

fn main() -> ExitCode {
    // Set when the alias exec'd a `pw2op` that turned out to be the alias
    // again (a copy, or a link that the OS resolved back to this file).
    let handed_on = std::env::var_os(ALIAS_ENV).is_some();
    std::env::remove_var(ALIAS_ENV);
    let named_pw2op = own_name().as_deref() == Some("pw2op");
    if handed_on || named_pw2op {
        eprintln!("{COPY_NOTE}");
        // Under its own name `pw2op`, a newer release's self-update replaces
        // this file with the real pw2op. Reached through a link, the running
        // file is the alias itself, so it updates as the alias.
        let file = if named_pw2op { "pw2op" } else { "gpm2op" };
        return pw2op::main("gpm2op", file);
    }
    #[cfg(unix)]
    if let Some(pw2op) = sibling() {
        use std::os::unix::process::CommandExt;
        // The pw2op it runs prints the note about the new name.
        let err = std::process::Command::new(&pw2op)
            .args(std::env::args_os().skip(1))
            .env(ALIAS_ENV, "gpm2op")
            .exec();
        eprintln!(
            "warning: could not run {}: {err}; running gpm2op's own copy.",
            pw2op.display()
        );
    }
    eprintln!("{INSTALL_NOTE}");
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
