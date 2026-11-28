//! `pw2op self-update` — replace the running binary with the latest GitHub
//! release build, via the family updater (`pk-cli-selfupdate`). Release
//! assets embed the Rust target triple, baked in by `build.rs`, and each
//! archive carries both `pw2op` and the `gpm2op` alias: the updater takes the
//! file named after the running binary. The alias runs an adjacent `pw2op`
//! when there is one, so there this is `pw2op`'s update.

use pk_cli_selfupdate::{SelfUpdateArgs, Updater};

use crate::error::{Error, Result};

pub fn run(args: &SelfUpdateArgs, json: bool, file: &str) -> Result<()> {
    updater(file)
        .run(args, json, false)
        .map_err(|e| Error::Update(e.to_string()))
}

/// The updater for the binary `bin`: it replaces the running executable with
/// the archive's file of that name.
fn updater(bin: &str) -> Updater {
    Updater {
        repo: "piekstra/pw2op-cli".into(),
        binary: bin.into(),
        target: env!("BUILD_TARGET").into(),
        current: env!("CARGO_PKG_VERSION").into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_name_updates_the_file_of_that_name() {
        for bin in ["pw2op", "gpm2op"] {
            let u = super::updater(bin);
            assert_eq!(u.binary, bin);
            assert_eq!(u.repo, "piekstra/pw2op-cli");
        }
    }
}
