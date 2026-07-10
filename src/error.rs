//! Error type.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "the 1Password CLI (`op`) is not installed or not on PATH.\n\
             Install it: https://developer.1password.com/docs/cli/get-started/"
    )]
    OpMissing,

    #[error("could not run `op`: {0}")]
    Spawn(String),

    #[error(
        "1Password CLI isn't signed in for this shell.\n\
         Fix it, then re-run:\n  \
         • Easiest: open the 1Password app → Settings → Developer → enable \
         \"Integrate with 1Password CLI\" (unlocks via Touch ID), or\n  \
         • Run:  eval $(op signin)"
    )]
    OpNotSignedIn,

    #[error("`op` reported an error:\n{0}")]
    Op(String),

    #[error("could not parse `op` output: {0}")]
    Parse(String),

    #[error("could not read the CSV export: {0}")]
    Csv(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("self-update failed: {0}")]
    Update(String),
}
