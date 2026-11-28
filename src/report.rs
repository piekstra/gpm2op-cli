//! Human and JSON rendering of sync results.
//!
//! Notes name rows by number, never by title, site, username or password.

use crate::credential::Origin;
use crate::source::Format;
use crate::sync::{Mode, Outcome, OutcomeKind, Summary, Unverified};
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Table};
use std::collections::BTreeSet;

/// One export of the run.
#[derive(Debug, Clone)]
pub struct Source {
    pub path: String,
    pub format: Format,
    /// Rows with a URL or a username, which were synced.
    pub rows: usize,
    /// Row numbers of rows with neither, which were not.
    pub unreadable_rows: Vec<usize>,
    pub deleted: bool,
}

/// What the notes and the JSON need to know about the run besides its counts.
pub struct RunReport<'a> {
    pub sources: &'a [Source],
    pub mode: Mode,
    pub tags: &'a [String],
}

/// The password managers the run's exports came from, for "still in …".
fn keepers(sources: &[Source]) -> String {
    let formats: BTreeSet<Format> = sources.iter().map(|s| s.format).collect();
    let names: Vec<&str> = formats.into_iter().map(Format::keeper).collect();
    if names.is_empty() {
        Format::Chrome.keeper().to_string()
    } else {
        names.join(" or ")
    }
}

/// The exports' paths, for a line that names them all.
pub fn paths(sources: &[Source]) -> String {
    sources
        .iter()
        .map(|s| s.path.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// "a.csv (Apple Passwords / Safari)" or, for several, each with its count.
pub fn read_from(sources: &[Source]) -> String {
    match sources {
        [one] => format!("{} ({})", one.path, one.format.label()),
        many => format!(
            "{} exports: {}",
            many.len(),
            many.iter()
                .map(|s| format!("{} ({}, {})", s.path, s.format.label(), s.rows))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// "row 3 of a.csv; rows 2, 5 of b.csv" for a set of row origins.
fn rows_text(origins: &[Origin], sources: &[Source]) -> String {
    let mut by_file: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut sorted = origins.to_vec();
    sorted.sort();
    for o in sorted {
        match by_file.last_mut() {
            Some((file, rows)) if *file == o.file => rows.push(o.row),
            _ => by_file.push((o.file, vec![o.row])),
        }
    }
    by_file
        .into_iter()
        .map(|(file, rows)| {
            let path = sources.get(file).map(|s| s.path.as_str()).unwrap_or("?");
            let list = rows
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let noun = if rows.len() == 1 { "row" } else { "rows" };
            format!("{noun} {list} of {path}")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn print_table(outcomes: &[Outcome], several_files: bool) {
    if outcomes.is_empty() {
        println!("(nothing to do)");
        return;
    }
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("Row"),
        Cell::new("Result"),
        Cell::new("Title"),
        Cell::new("Host"),
        Cell::new("Username"),
    ]);
    for o in outcomes {
        let row = if several_files {
            format!("{}:{}", o.origin.file + 1, o.origin.row)
        } else {
            o.origin.row.to_string()
        };
        table.add_row(vec![
            Cell::new(row),
            Cell::new(label(&o.result)),
            Cell::new(&o.title),
            Cell::new(&o.host),
            Cell::new(&o.username),
        ]);
    }
    println!("{table}");
}

pub fn print_summary(summary: &Summary, dry_run: bool) {
    if dry_run {
        println!(
            "Planned: {} change(s) across the export (nothing was written).",
            summary.planned
        );
        return;
    }
    println!(
        "Done: {} created, {} updated, {} unchanged, {} left as-is, {} failed.",
        summary.created,
        summary.updated,
        summary.unchanged,
        summary.skipped_existing,
        summary.failed
    );
}

/// The `sync-result/v1` document. Each item names its row by number only;
/// `verbose` (`-v`) adds the row's `title`, `host` and `username`.
pub fn print_json(outcomes: &[Outcome], summary: &Summary, run: &RunReport, verbose: bool) {
    let items: Vec<_> = outcomes
        .iter()
        .map(|o| {
            let mut item = serde_json::json!({
                "file": o.origin.file,
                "row": o.origin.row,
                "result": label(&o.result),
            });
            if verbose {
                item["title"] = o.title.clone().into();
                item["host"] = o.host.clone().into();
                item["username"] = o.username.clone().into();
            }
            item
        })
        .collect();
    let formats: BTreeSet<&str> = run.sources.iter().map(|s| s.format.id()).collect();
    let format = match formats.len() {
        1 => formats.into_iter().next().unwrap_or_default(),
        _ => "mixed",
    };
    let sources: Vec<_> = run
        .sources
        .iter()
        .map(|s| {
            serde_json::json!({
                "path": s.path,
                "format": s.format.id(),
                "rows": s.rows,
                "unreadable_rows": s.unreadable_rows,
                "deleted": s.deleted,
            })
        })
        .collect();
    let ignored: Vec<_> = summary
        .otp
        .ignored
        .iter()
        .map(|o| serde_json::json!({"file": o.file, "row": o.row}))
        .collect();
    let doc = serde_json::json!({
        "schema": "sync-result/v1",
        "summary": {
            "format": format,
            "mode": run.mode.id(),
            "tags": run.tags,
            "created": summary.created,
            "updated": summary.updated,
            "unchanged": summary.unchanged,
            "skipped_existing": summary.skipped_existing,
            "failed": summary.failed,
            "planned": summary.planned,
            "repeats": summary.repeats,
            "conflicting_logins": summary.conflicting_logins,
            "otp": {
                "added": summary.otp.added,
                "not_added": summary.otp.not_added,
                "ignored_rows": ignored,
            },
            "notes_not_added": summary.notes_not_added,
            "csv_deleted": !run.sources.is_empty() && run.sources.iter().all(|s| s.deleted),
            "sources": sources,
            "unverified": {
                "not_compared": summary.unverified.not_compared,
                "host_only": summary.unverified.host_only,
                "title_only": summary.unverified.title_only,
                "deduped": summary.unverified.deduped,
                "unreadable": summary.unverified.unreadable,
            },
        },
        "items": items,
    });
    pk_cli_core::output::json(&doc);
}

/// Say on stderr which rows were not checked against 1Password, and what
/// keep-all and the one-time passwords came to.
pub fn print_notes(summary: &Summary, run: &RunReport) {
    for note in notes(summary, run) {
        eprintln!("{note}");
    }
}

fn notes(summary: &Summary, run: &RunReport) -> Vec<String> {
    let keeper = keepers(run.sources);
    let mut notes = unverified_notes(&summary.unverified, &keeper, run.sources);
    if summary.repeats > 0 {
        notes.push(format!(
            "{} row(s) repeated an earlier row's site, username and password exactly, across the \
             exports of this run, and were imported once.",
            summary.repeats
        ));
    }
    if summary.conflicting_logins > 0 {
        let tagged = match run.tags {
            [] => String::new(),
            tags => format!(
                " The logins this run created are tagged {}; one that was already in 1Password \
                 is not.",
                tags.join(", ")
            ),
        };
        let failed = if summary.failed > 0 {
            " Rows that failed were not created."
        } else {
            ""
        };
        notes.push(format!(
            "{} site and username pair(s) have more than one password across the exports, and \
             each password is kept as its own login.{tagged}{failed} Search 1Password by site and \
             username to see every candidate, sign in to find the current one, then delete the \
             others.",
            summary.conflicting_logins
        ));
    }
    if summary.notes_not_added > 0 {
        notes.push(format!(
            "{} row(s) had a note for a login that already exists in 1Password, and it was not \
             added to that login. It is still in {keeper}.",
            summary.notes_not_added
        ));
    }
    if summary.otp.not_added > 0 {
        notes.push(format!(
            "{} row(s) carried a one-time-password secret for a login that already exists in \
             1Password, and it was not added to that login. It is still in {keeper}.",
            summary.otp.not_added
        ));
    }
    if !summary.otp.ignored.is_empty() {
        notes.push(format!(
            "{} row(s) had an OTPAuth value that is not an otpauth:// address ({}), so no \
             one-time password was imported for them. It is still in {keeper}.",
            summary.otp.ignored.len(),
            rows_text(&summary.otp.ignored, run.sources)
        ));
    }
    notes
}

fn unverified_notes(u: &Unverified, keeper: &str, sources: &[Source]) -> Vec<String> {
    let mut notes = Vec::new();
    if u.not_compared > 0 {
        notes.push(format!(
            "{} row(s) matched an existing login and were not compared. Their passwords are \
             still in {keeper}. --update --dry-run -v lists the logins --update would compare; \
             --update --force reconciles them (it overwrites a passkey on a login it edits, see \
             the README's Passkeys section), or --keep-all keeps both passwords.",
            u.not_compared
        ));
    }
    if u.host_only > 0 {
        notes.push(format!(
            "{} row(s) had no username and matched the only login on their site on host alone, \
             which may be another account, so that login was not updated, even with --update. \
             Check those logins in 1Password; the rows' passwords are still in {keeper}.",
            u.host_only
        ));
    }
    if u.title_only > 0 {
        notes.push(format!(
            "{} row(s) had no URL and matched a login with no URL on title and username alone, \
             which may be another device's or account's, so that login was not updated, even \
             with --update. Check those logins in 1Password; the rows' passwords are still in \
             {keeper}.",
            u.title_only
        ));
    }
    if u.deduped > 0 {
        notes.push(format!(
            "{} row(s) repeated a later row's site and username, and only the later row was \
             synced, with any password, code or note it lacked taken from the earlier row. The \
             earlier passwords and codes are still in {keeper}; --keep-all imports every \
             password.",
            u.deduped
        ));
    }
    if u.unreadable > 0 {
        let origins: Vec<Origin> = sources
            .iter()
            .enumerate()
            .flat_map(|(file, s)| {
                s.unreadable_rows
                    .iter()
                    .map(move |&row| Origin { file, row })
            })
            .collect();
        let rows = if origins.is_empty() {
            String::new()
        } else {
            format!(" ({})", rows_text(&origins, sources))
        };
        notes.push(format!(
            "{} row(s) had no URL and no username{rows}, so they were not synced. Any password \
             in them is still in {keeper}.",
            u.unreadable
        ));
    }
    notes
}

pub fn print_check_json(vault: &str, logins: usize) {
    pk_cli_core::output::json(&serde_json::json!({
        "schema": "check-result/v1",
        "vault": vault,
        "logins": logins,
    }));
}

/// The human line for an export that was deleted. It says 1Password holds
/// everything only when `Summary::complete`.
pub fn deleted_line(source: &Source, summary: &Summary) -> String {
    let csv = &source.path;
    let keeper = source.format.keeper();
    if summary.complete() {
        format!("Deleted the export at {csv}: every credential read from it is in 1Password now.")
    } else {
        format!(
            "Deleted the export at {csv}. {keeper} still holds every password in it, including \
             what the notes above say did not reach 1Password."
        )
    }
}

fn label(kind: &OutcomeKind) -> String {
    match kind {
        OutcomeKind::Created => "created".into(),
        OutcomeKind::Updated => "updated".into(),
        OutcomeKind::Unchanged => "unchanged".into(),
        OutcomeKind::SkippedExisting => "exists".into(),
        OutcomeKind::Failed(e) => format!("failed: {e}"),
        OutcomeKind::Planned(p) => format!("would {p}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::OnConflict;

    fn source(path: &str, format: Format) -> Source {
        Source {
            path: path.into(),
            format,
            rows: 1,
            unreadable_rows: vec![],
            deleted: true,
        }
    }

    fn run(sources: &[Source]) -> RunReport<'_> {
        RunReport {
            sources,
            mode: Mode::Upsert(OnConflict::Skip),
            tags: &[],
        }
    }

    #[test]
    fn every_unverified_group_gets_its_own_note() {
        let chrome = [source("x.csv", Format::Chrome)];
        assert!(notes(&Summary::default(), &run(&chrome)).is_empty());
        let summary = Summary {
            unverified: Unverified {
                not_compared: 3,
                host_only: 2,
                title_only: 5,
                deduped: 1,
                unreadable: 4,
            },
            ..Summary::default()
        };
        let notes = notes(&summary, &run(&chrome));
        assert_eq!(notes.len(), 5);
        assert!(notes[0].starts_with("3 row(s) matched an existing login and were not compared"));
        assert!(notes[0].contains("--update --force reconciles them"));
        assert!(notes[0].contains("still in Google Password Manager"));
        assert!(notes[1].starts_with("2 row(s) had no username"));
        assert!(
            notes[2].starts_with("5 row(s) had no URL and matched a login with no URL on title")
        );
        assert!(notes[3].starts_with("1 row(s) repeated a later row's"));
        assert!(notes[4].starts_with("4 row(s) had no URL and no username"));
    }

    #[test]
    fn notes_name_the_password_manager_each_export_came_from() {
        let summary = Summary {
            unverified: Unverified {
                not_compared: 1,
                ..Unverified::default()
            },
            ..Summary::default()
        };
        let apple = [source("a.csv", Format::Apple)];
        assert!(notes(&summary, &run(&apple))[0].contains("still in Apple Passwords."));
        let both = [
            source("b.csv", Format::Chrome),
            source("a.csv", Format::Apple),
        ];
        assert!(notes(&summary, &run(&both))[0]
            .contains("still in Google Password Manager or Apple Passwords."));
    }

    #[test]
    fn rows_are_named_by_number_per_file() {
        let sources = [
            source("a.csv", Format::Apple),
            source("b.csv", Format::Chrome),
        ];
        let origins = [
            Origin { file: 1, row: 2 },
            Origin { file: 0, row: 7 },
            Origin { file: 0, row: 3 },
        ];
        assert_eq!(
            rows_text(&origins, &sources),
            "rows 3, 7 of a.csv; row 2 of b.csv"
        );
    }

    #[test]
    fn keep_all_and_otp_notes() {
        let sources = [source("a.csv", Format::Apple)];
        let summary = Summary {
            repeats: 2,
            conflicting_logins: 1,
            otp: crate::sync::OtpCounts {
                added: 1,
                not_added: 1,
                ignored: vec![Origin { file: 0, row: 4 }],
            },
            ..Summary::default()
        };
        let tags = ["import-1".to_string()];
        let report = RunReport {
            sources: &sources,
            mode: Mode::KeepAll,
            tags: &tags,
        };
        let notes = notes(&summary, &report);
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(notes[0]
            .starts_with("2 row(s) repeated an earlier row's site, username and password exactly"));
        assert!(notes[1].contains("The logins this run created are tagged import-1; one that was already in 1Password is not."));
        assert!(notes[2].contains("It is still in Apple Passwords."));
        assert!(notes[3].contains("(row 4 of a.csv)"));
    }

    #[test]
    fn the_deleted_line_claims_1password_only_when_everything_reached_it() {
        let chrome = source("x.csv", Format::Chrome);
        let all = deleted_line(&chrome, &Summary::default());
        assert!(all.contains("every credential read from it is in 1Password now"));
        let partial = [
            Summary {
                unverified: Unverified {
                    not_compared: 1,
                    ..Unverified::default()
                },
                ..Summary::default()
            },
            Summary {
                otp: crate::sync::OtpCounts {
                    not_added: 1,
                    ..Default::default()
                },
                ..Summary::default()
            },
            Summary {
                otp: crate::sync::OtpCounts {
                    ignored: vec![Origin { file: 0, row: 2 }],
                    ..Default::default()
                },
                ..Summary::default()
            },
            Summary {
                notes_not_added: 1,
                ..Summary::default()
            },
            Summary {
                unverified: Unverified {
                    title_only: 1,
                    ..Unverified::default()
                },
                ..Summary::default()
            },
        ];
        for summary in &partial {
            let some = deleted_line(&chrome, summary);
            assert!(!some.contains("is in 1Password now"), "{some}");
            assert!(some.contains("Google Password Manager still holds every password"));
        }
        let apple = deleted_line(&source("a.csv", Format::Apple), &partial[0]);
        assert!(
            apple.contains("Apple Passwords still holds every password"),
            "{apple}"
        );
    }
}
