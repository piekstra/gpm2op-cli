//! `gpm2op`, the name pw2op had through v0.4.0. It runs pw2op as `gpm2op`, so
//! scripts keep working and a 0.4.0 install's `self-update`, which replaces
//! the `gpm2op` binary from the release archive, lands here. Kept for at
//! least two releases after the rename (through v0.6.x).

fn main() -> std::process::ExitCode {
    eprintln!("note: gpm2op is now pw2op; the gpm2op command keeps working for now.");
    pw2op::main("gpm2op")
}
