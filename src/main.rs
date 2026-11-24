//! gpm2op — upsert a Google/Chrome Password Manager export into 1Password.

mod chrome;
mod cli;
mod credential;
mod error;
mod export;
mod op;
mod report;
mod selfupdate;
mod sync;

use clap::Parser;
use cli::{CheckArgs, Cli, Command, SyncArgs};
use credential::Credential;
use op::Op;
use std::path::Path;
use std::process::ExitCode;
use sync::OnConflict;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json_mode = cli.json;
    match run(cli) {
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
        _ => CliError::Other(err.to_string()),
    }
}

fn run(cli: Cli) -> error::Result<ExitCode> {
    let json = cli.json;
    match cli.command {
        Command::Check(args) => check(args, json),
        Command::Sync(mut args) => {
            args.json |= json;
            do_sync(args)
        }
        Command::SelfUpdate(args) => selfupdate::run(&args, json).map(|()| ExitCode::SUCCESS),
        Command::Completions { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "gpm2op", &mut std::io::stdout());
            Ok(ExitCode::SUCCESS)
        }
        Command::Info => {
            use pk_cli_core::info::{AuthInfo, CliInfo};
            let info = CliInfo::new(
                "gpm2op",
                env!("CARGO_PKG_VERSION"),
                "https://github.com/piekstra/gpm2op-cli",
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
    let op = Op::new(args.account.clone());
    // Fail early with a clear message if op isn't usable.
    op.preflight()?;

    let path = Path::new(&args.csv);
    let read = export::Snapshot::of(path);
    let rows = chrome::read_csv(path)?;
    let creds: Vec<Credential> = rows.into_iter().map(Credential::from).collect();
    let total = creds.len();
    let end = |failed| export::RunEnd {
        dry_run: args.dry_run,
        keep_csv: args.keep_csv,
        credentials: total,
        failed,
    };

    if creds.is_empty() {
        let disposal = export::dispose(path, read.as_ref(), end(0));
        if args.json {
            report::print_json(&[], &sync::Summary::default(), disposal.deleted());
        } else {
            println!("No usable credentials found in {}.", args.csv);
        }
        note_kept(&args.csv, &disposal);
        return Ok(ExitCode::SUCCESS);
    }

    let on_conflict = if args.update {
        OnConflict::Update
    } else {
        OnConflict::Skip
    };

    eprintln!(
        "Read {total} credential(s) from {}. Target vault: \"{}\"{}.",
        args.csv,
        args.vault,
        if args.dry_run { " (dry run)" } else { "" }
    );

    let (outcomes, summary) = sync::run(&op, &args.vault, creds, on_conflict, args.dry_run)?;
    let disposal = export::dispose(path, read.as_ref(), end(summary.failed));

    if args.json {
        report::print_json(&outcomes, &summary, disposal.deleted());
    } else {
        if args.verbose || args.dry_run {
            report::print_table(&outcomes);
        }
        report::print_summary(&summary, args.dry_run);
        if disposal.deleted() {
            println!(
                "Deleted the export at {}: every row in it is in 1Password now.",
                args.csv
            );
        }
    }
    note_kept(&args.csv, &disposal);

    Ok(if summary.failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Say on stderr why the export is still on disk, in every output mode.
fn note_kept(csv: &str, disposal: &export::Disposal) {
    if let export::Disposal::Kept(kept) = disposal {
        if let Some(note) = export::kept_note(csv, kept) {
            eprintln!("{note}");
        }
    }
}
