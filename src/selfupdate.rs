//! `gpm2op self-update` — replace the running binary with the latest GitHub
//! release build, via the family updater (`pk-cli-selfupdate`). Release
//! assets embed the Rust target triple, baked in by `build.rs`.

use pk_cli_selfupdate::{SelfUpdateArgs, Updater};

use crate::error::{Error, Result};

pub fn run(args: &SelfUpdateArgs, json: bool) -> Result<()> {
    Updater {
        repo: "piekstra/gpm2op-cli".into(),
        binary: "gpm2op".into(),
        target: env!("BUILD_TARGET").into(),
        current: env!("CARGO_PKG_VERSION").into(),
    }
    .run(args, json, false)
    .map_err(|e| Error::Update(e.to_string()))
}
