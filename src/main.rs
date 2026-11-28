//! `pw2op`: see the library crate.

/// Set by the `gpm2op` alias when it execs this binary: the name the user
/// typed, and the mark that stops a `pw2op` that is itself a copy of the alias
/// from execing again.
pub const ALIAS_ENV: &str = "PW2OP_INVOKED_AS";

fn main() -> std::process::ExitCode {
    let name = match std::env::var(ALIAS_ENV).as_deref() {
        Ok("gpm2op") => "gpm2op",
        _ => "pw2op",
    };
    // Not passed on to `op` or anything else this run starts.
    std::env::remove_var(ALIAS_ENV);
    if name == "gpm2op" {
        eprintln!("note: gpm2op is now pw2op. Call pw2op instead: gpm2op is removed after v0.6.x.");
    }
    pw2op::main(name, "pw2op")
}
