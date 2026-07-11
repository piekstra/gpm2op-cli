//! Command-line surface.

use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;
pub use pk_cli_selfupdate::SelfUpdateArgs;

/// Catch your 1Password vault up to a Google/Chrome Password Manager export.
///
/// `gpm2op` reads a Chrome/Google passwords CSV and idempotently upserts each
/// login into 1Password via the `op` CLI: it creates the entries you don't have
/// yet and, by default, leaves everything else untouched. Run it as often as you
/// like — re-running only adds what's newly appeared.
#[derive(Debug, Parser)]
#[command(name = "gpm2op", version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout (diagnostics go to stderr).
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Upsert a Chrome/Google CSV export into 1Password.
    Sync(SyncArgs),
    /// Verify `op` is installed and signed in, and show the target vault.
    Check(CheckArgs),
    /// Update gpm2op in place from the latest GitHub release.
    SelfUpdate(SelfUpdateArgs),

    /// Print a shell completion script (e.g. `gpm2op completions zsh`).
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Machine-readable capability discovery (cli-info/v1).
    Info,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Path to the Chrome/Google Password Manager CSV export.
    #[arg(value_name = "CSV")]
    pub csv: String,

    /// 1Password vault to write into.
    #[arg(long, default_value = "Private")]
    pub vault: String,

    /// Also update the stored password when an existing login's password
    /// differs from the CSV (default: leave existing logins untouched).
    #[arg(long)]
    pub update: bool,

    /// Show what would happen without writing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Operate on a specific 1Password account (shorthand, sign-in address, or
    /// account ID). Omit if you have a single account.
    #[arg(long)]
    pub account: Option<String>,

    /// Print the per-item results table (otherwise just the summary).
    #[arg(short, long)]
    pub verbose: bool,

    /// Emit the summary (and results) as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// 1Password vault to check access to.
    #[arg(long, default_value = "Private")]
    pub vault: String,
    /// Operate on a specific 1Password account.
    #[arg(long)]
    pub account: Option<String>,
}
