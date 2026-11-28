//! pw2op (formerly gpm2op) — upsert Google/Chrome and Apple Passwords/Safari
//! exports into 1Password. The `pw2op` binary calls [`main`]; the `gpm2op`
//! alias calls it only when no `pw2op` is beside it, and otherwise execs that
//! `pw2op`, which then answers as `gpm2op`.

mod cli;
mod credential;
mod error;
mod export;
mod op;
mod report;
mod selfupdate;
mod source;
mod sync;

use clap::{CommandFactory, FromArgMatches};
use cli::{CheckArgs, Cli, Command, SyncArgs};
use credential::{is_otpauth, Credential, Origin};
use op::Op;
use report::{RunReport, Source};
use std::path::Path;
use std::process::ExitCode;
use sync::{Mode, OnConflict};

/// Run the CLI. `name` is the command the user typed (`pw2op` or `gpm2op`),
/// used in help and completions; `file` is the executable that is running,
/// the one `self-update` replaces. They differ when the alias execs `pw2op`.
pub fn main(name: &'static str, file: &'static str) -> ExitCode {
    let matches = Cli::command().name(name).bin_name(name).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let json_mode = cli.json;
    match run(cli, name, file) {
        Ok(code) => code,
        Err(err) => {
            let cli_err = to_cli_error(&err);
            if json_mode {
                pk_cli_core::output::json(&cli_err.to_json());
            }
            eprintln!("error: {err}");
            ExitCode::from(cli_err.exit_code() as u8)
        }
    }
}

/// Map local errors onto the family exit-code contract (piekstra-cli/1).
fn to_cli_error(err: &error::Error) -> pk_cli_core::CliError {
    use error::Error as E;
    use pk_cli_core::CliError;
    match err {
        E::OpNotSignedIn => CliError::Auth(err.to_string()),
        E::OpMissing | E::Csv(_) => CliError::Usage(err.to_string()),
        E::Op(m) | E::Parse(m) => CliError::Upstream(m.clone()),
        E::Update(m) => CliError::Other(m.clone()),
        E::Confirm(m) => CliError::ConfirmationRequired(m.clone()),
        _ => CliError::Other(err.to_string()),
    }
}

fn run(cli: Cli, name: &'static str, file: &'static str) -> error::Result<ExitCode> {
    let json = cli.json;
    match cli.command {
        Command::Check(args) => check(args, json),
        Command::Sync(mut args) => {
            args.json |= json;
            do_sync(args)
        }
        Command::SelfUpdate(args) => selfupdate::run(&args, json, file).map(|()| ExitCode::SUCCESS),
        Command::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command().name(name),
                name,
                &mut std::io::stdout(),
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Info => {
            use pk_cli_core::info::{AuthInfo, CliInfo};
            let info = CliInfo::new(
                "pw2op",
                env!("CARGO_PKG_VERSION"),
                "https://github.com/piekstra/pw2op-cli",
                AuthInfo {
                    required: false,
                    method: "none".into(),
                    login_hint: Some("sign in to the 1Password CLI (`op signin`)".into()),
                },
                &["sync", "check"],
            );
            pk_cli_core::output::json(&serde_json::to_value(&info).unwrap_or_default());
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn check(args: CheckArgs, json: bool) -> error::Result<ExitCode> {
    let op = Op::new(args.account);
    op.preflight()?;
    let logins = op.list_logins(&args.vault)?;
    if json {
        report::print_check_json(&args.vault, logins.len());
    } else {
        println!(
            "✓ op is installed and signed in. Vault \"{}\" has {} login item(s).",
            args.vault,
            logins.len()
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn do_sync(args: SyncArgs) -> error::Result<ExitCode> {
    // Every export is read, and its format decided, before `op` is touched,
    // so a wrong file costs no 1Password prompt.
    reject_repeated_paths(&args.csv)?;
    if args.update && !args.dry_run && !args.force {
        return Err(error::Error::Confirm(
            "--update rewrites each changed login from a JSON template, and op overwrites a \
             passkey stored on the login, which op's item JSON does not reliably show. Rerun with \
             --force once those logins hold no passkey you need (or before passkeys are moved \
             into 1Password). --update --dry-run -v lists every matched login --update would \
             compare and might rewrite; it checks no passkeys, and reads passwords only to \
             compare rows with neither a URL nor a title, which are never rewritten."
                .into(),
        ));
    }
    let mut sources = Vec::with_capacity(args.csv.len());
    let mut snapshots = Vec::with_capacity(args.csv.len());
    let mut creds: Vec<Credential> = Vec::new();
    let mut otp_ignored = Vec::new();
    for (file, csv) in args.csv.iter().enumerate() {
        let path = Path::new(csv);
        snapshots.push(export::Snapshot::of(path));
        let export = source::read_csv(path)?;
        for row in &export.rows {
            if !row.otp.trim().is_empty() && !is_otpauth(&row.otp) {
                otp_ignored.push(Origin { file, row: row.row });
            }
        }
        sources.push(Source {
            path: csv.clone(),
            format: export.format,
            rows: export.rows.len(),
            unreadable_rows: export.unreadable,
            deleted: false,
        });
        creds.extend(
            export
                .rows
                .into_iter()
                .map(|row| Credential::from_row(file, row)),
        );
    }

    let mode = if args.keep_all {
        Mode::KeepAll
    } else if args.update {
        Mode::Upsert(OnConflict::Update)
    } else {
        Mode::Upsert(OnConflict::Skip)
    };
    let total = creds.len();
    let unreadable: usize = sources.iter().map(|s| s.unreadable_rows.len()).sum();

    let (outcomes, mut summary) = if creds.is_empty() {
        (Vec::new(), sync::Summary::default())
    } else {
        let op = Op::new(args.account.clone());
        // Fail early with a clear message if op isn't usable.
        op.preflight()?;
        eprintln!(
            "Read {total} credential(s) from {}. Target vault: \"{}\"{}{}.",
            report::read_from(&sources),
            args.vault,
            if args.keep_all {
                ", keeping every password"
            } else {
                ""
            },
            if args.dry_run { " (dry run)" } else { "" }
        );
        let opts = sync::Options {
            vault: &args.vault,
            mode,
            dry_run: args.dry_run,
            tags: &args.tags,
        };
        sync::run(&op, &opts, creds)?
    };
    summary.unverified.unreadable = unreadable;
    summary.otp.ignored = otp_ignored;

    // A failed row anywhere keeps every export: the rows of one run are
    // matched and de-duplicated together, so a rerun needs them all.
    let mut kept = Vec::new();
    for (source, read) in sources.iter_mut().zip(&snapshots) {
        let end = export::RunEnd {
            dry_run: args.dry_run,
            keep_csv: args.keep_csv,
            credentials: source.rows,
            failed: summary.failed,
        };
        match export::dispose(Path::new(&source.path), read.as_ref(), end) {
            export::Disposal::Deleted => source.deleted = true,
            export::Disposal::Kept(why) => kept.push((source.path.clone(), why)),
        }
    }

    let run = RunReport {
        sources: &sources,
        mode,
        tags: &args.tags,
    };
    if args.json {
        report::print_json(&outcomes, &summary, &run, args.verbose);
    } else if total == 0 {
        println!(
            "No usable credentials found in {}.",
            report::paths(&sources)
        );
        report::print_notes(&summary, &run);
    } else {
        if args.verbose {
            report::print_table(&outcomes, sources.len() > 1);
        }
        report::print_summary(&summary, args.dry_run);
        if !args.dry_run {
            report::print_notes(&summary, &run);
        }
        for source in sources.iter().filter(|s| s.deleted) {
            println!("{}", report::deleted_line(source, &summary));
        }
    }
    for (path, why) in &kept {
        if let Some(note) = export::kept_note(path, why) {
            eprintln!("{note}");
        }
    }

    Ok(if summary.failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Refuse an export named twice: its rows would count as repeats of
/// themselves, and the second disposal would find the first one's work.
fn reject_repeated_paths(paths: &[String]) -> error::Result<()> {
    let mut seen = std::collections::HashSet::new();
    for (i, raw) in paths.iter().enumerate() {
        let key = std::fs::canonicalize(raw).unwrap_or_else(|_| raw.into());
        if !seen.insert(key) {
            return Err(error::Error::Csv(format!(
                "{raw} is named more than once (argument {}); name each export once",
                i + 1
            )));
        }
    }
    Ok(())
}
