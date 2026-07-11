//! gpm2op — upsert a Google/Chrome Password Manager export into 1Password.

mod chrome;
mod cli;
mod credential;
mod error;
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
        Command::Check(args) => check(args),
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
