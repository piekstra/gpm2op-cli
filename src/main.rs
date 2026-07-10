//! gpm2op — upsert a Google/Chrome Password Manager export into 1Password.

mod chrome;
mod cli;
mod credential;
mod error;
mod op;
mod report;
mod sync;

use clap::Parser;
use cli::{CheckArgs, Cli, Command, SyncArgs};
use credential::Credential;
use op::Op;
use std::path::Path;
use std::process::ExitCode;
use sync::OnConflict;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> error::Result<ExitCode> {
    match cli.command {
        Command::Check(args) => check(args),
        Command::Sync(args) => do_sync(args),
    }
}

fn check(args: CheckArgs) -> error::Result<ExitCode> {
    let op = Op::new(args.account);
    op.preflight()?;
    let logins = op.list_logins(&args.vault)?;
    println!(
        "✓ op is installed and signed in. Vault \"{}\" has {} login item(s).",
        args.vault,
        logins.len()
    );
    Ok(ExitCode::SUCCESS)
}

fn do_sync(args: SyncArgs) -> error::Result<ExitCode> {
    let op = Op::new(args.account.clone());
    // Fail early with a clear message if op isn't usable.
    op.preflight()?;

    let rows = chrome::read_csv(Path::new(&args.csv))?;
    let creds: Vec<Credential> = rows.into_iter().map(Credential::from).collect();
    if creds.is_empty() {
        println!("No usable credentials found in {}.", args.csv);
        return Ok(ExitCode::SUCCESS);
    }

    let on_conflict = if args.update {
        OnConflict::Update
    } else {
        OnConflict::Skip
    };

    let total = creds.len();
    println!(
        "Read {total} credential(s) from {}. Target vault: \"{}\"{}.",
        args.csv,
        args.vault,
        if args.dry_run { " (dry run)" } else { "" }
    );

    let (outcomes, summary) = sync::run(&op, &args.vault, creds, on_conflict, args.dry_run)?;

    if args.json {
        report::print_json(&outcomes, &summary);
    } else {
        if args.verbose || args.dry_run {
            report::print_table(&outcomes);
        }
        report::print_summary(&summary, args.dry_run);
        if !args.dry_run && summary.created > 0 {
            eprintln!(
                "\nReminder: the CSV export at {} contains your passwords in plain text. \
                 Delete it now that they're in 1Password.",
                args.csv
            );
        }
    }

    Ok(if summary.failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
