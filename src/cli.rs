//! Command-line surface.

use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;
pub use pk_cli_selfupdate::SelfUpdateArgs;

/// Catch your 1Password vault up to a Google/Chrome or Apple Passwords/Safari export.
///
/// `pw2op` reads a Chrome/Google or Apple Passwords/Safari passwords CSV and
/// idempotently upserts each login into 1Password via the `op` CLI: it creates
/// the entries you don't have yet and, by default, leaves everything else
/// untouched. Run it as often as you like — re-running only adds what's newly
/// appeared.
#[derive(Debug, Parser)]
#[command(name = "pw2op", version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout (diagnostics go to stderr).
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Upsert Chrome/Google or Apple Passwords/Safari CSV exports into 1Password.
    Sync(SyncArgs),
    /// Verify `op` is installed and signed in, and show the target vault.
    Check(CheckArgs),
    /// Update pw2op in place from the latest GitHub release.
    SelfUpdate(SelfUpdateArgs),

    /// Print a shell completion script (e.g. `pw2op completions zsh`).
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
    /// One or more CSV exports: Chrome / Google Password Manager
    /// (name,url,username,password,note) or Apple Passwords / Safari
    /// (Title,URL,Username,Password,Notes,OTPAuth), told apart by the header.
    /// Each is deleted after a sync with no failures (see --keep-csv).
    #[arg(value_name = "CSV", required = true, num_args = 1..)]
    pub csv: Vec<String>,

    /// 1Password vault to write into.
    #[arg(long, default_value = "Private")]
    pub vault: String,

    /// Also update the stored password when an existing login's password
    /// differs from the CSV (default: leave existing logins untouched). Rows
    /// matched on host alone (no username) or on title alone (no URL) are
    /// never updated.
    /// Needs --force outside a dry run: op's template edit overwrites a
    /// passkey stored on the login.
    #[arg(long, conflicts_with = "keep_all")]
    pub update: bool,

    /// Let --update edit existing logins, accepting that op's template edit
    /// overwrites a passkey stored on one. pw2op still refuses a login whose
    /// item JSON shows a passkey, but op does not reliably show one.
    #[arg(long, requires = "update")]
    pub force: bool,

    /// Create a login for every distinct site, username and password, for
    /// exports that disagree about which password is current. Repeats across
    /// the files of the run are imported once (a different one-time password
    /// keeps a repeat as its own login), and a login already in the vault
    /// with the same site, username and password is not created again (its
    /// password is read to compare). Nothing existing is changed.
    /// Without it, one login per site and username, the last row winning.
    #[arg(long)]
    pub keep_all: bool,

    /// Tag every login this run creates (repeat for more than one), so the
    /// import can be found, and undone, in 1Password.
    #[arg(long = "tag", value_name = "TAG", value_parser = parse_tag)]
    pub tags: Vec<String>,

    /// Show what would happen without writing anything. The export is kept.
    #[arg(long)]
    pub dry_run: bool,

    /// Keep the CSV export on disk after the sync. By default it is deleted
    /// once a run finishes with no failed rows, since it holds every password
    /// in plain text.
    #[arg(long)]
    pub keep_csv: bool,

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

/// A tag `op item create --tags` takes as one tag: not blank, no comma.
fn parse_tag(raw: &str) -> Result<String, String> {
    let tag = raw.trim();
    if tag.is_empty() {
        return Err("a tag cannot be blank".into());
    }
    if tag.contains(',') {
        return Err("a tag cannot contain a comma; repeat --tag for each tag".into());
    }
    Ok(tag.to_string())
}
