//! `gpm2op self-update` — replace the running binary with the latest GitHub
//! release build.
//!
//! Compares the compiled-in version against the newest release and (unless
//! `--check`) downloads the current platform's asset and swaps the binary in
//! place. Installed via a package manager? Prefer that manager's upgrade path.

use crate::cli::SelfUpdateArgs;
use crate::error::{Error, Result};

const REPO_OWNER: &str = "piekstra";
const REPO_NAME: &str = "gpm2op";
const BIN_NAME: &str = "gpm2op";

pub fn run(args: &SelfUpdateArgs) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    let updater = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(current)
        .show_download_progress(!args.check)
        .no_confirm(args.yes)
        .build()
        .map_err(|e| Error::Update(e.to_string()))?;

    if args.check {
        let latest = updater
            .get_latest_release()
            .map_err(|e| Error::Update(e.to_string()))?;
        if self_update::version::bump_is_greater(current, &latest.version).unwrap_or(false) {
            println!("Update available: {current} → {}", latest.version);
            println!("Run `gpm2op self-update` to install it.");
        } else {
            println!("gpm2op is up to date ({current}).");
        }
        return Ok(());
    }

    let status = updater.update().map_err(|e| Error::Update(e.to_string()))?;
    if status.updated() {
        println!("Updated gpm2op {current} → {}.", status.version());
    } else {
        println!("gpm2op is already up to date ({}).", status.version());
    }
    Ok(())
}
