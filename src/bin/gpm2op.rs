//! `gpm2op`, the name pw2op had through v0.4.0, kept through v0.6.x.
//!
//! With `pw2op` installed beside it, the alias runs that binary, so both names
//! always run the same release: `self-update` through either updates `pw2op`,
//! which `gpm2op` then runs. On its own (a 0.4.0 install that self-updated,
//! whose updater extracts only `gpm2op`) it runs the bundled copy of the code.

use std::process::ExitCode;

const NOTE: &str = "note: gpm2op is now pw2op. Install pw2op beside this command \
(https://github.com/piekstra/pw2op-cli#renamed-from-gpm2op) and gpm2op runs it; \
gpm2op itself is removed after v0.6.x.";

fn main() -> ExitCode {
    eprintln!("{NOTE}");
    #[cfg(unix)]
    if let Some(pw2op) = sibling() {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&pw2op)
            .args(std::env::args_os().skip(1))
            .exec();
        eprintln!(
            "warning: could not run {}: {err}; running gpm2op's own copy.",
            pw2op.display()
        );
    }
    pw2op::main("gpm2op")
}

/// The `pw2op` executable in this binary's own directory, if there is one.
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
