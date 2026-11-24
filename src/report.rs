//! Human and JSON rendering of sync results.

use crate::sync::{Outcome, OutcomeKind, Summary, Unverified};
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Table};

pub fn print_table(outcomes: &[Outcome]) {
    if outcomes.is_empty() {
        println!("(nothing to do)");
        return;
    }
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("Result"),
        Cell::new("Title"),
        Cell::new("Host"),
        Cell::new("Username"),
    ]);
    for o in outcomes {
        table.add_row(vec![
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

pub fn print_json(outcomes: &[Outcome], summary: &Summary, csv_deleted: bool) {
    let items: Vec<_> = outcomes
        .iter()
        .map(|o| {
            serde_json::json!({
                "result": label(&o.result),
                "title": o.title,
                "host": o.host,
                "username": o.username,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "schema": "sync-result/v1",
        "summary": {
            "created": summary.created,
            "updated": summary.updated,
            "unchanged": summary.unchanged,
            "skipped_existing": summary.skipped_existing,
            "failed": summary.failed,
            "planned": summary.planned,
            "csv_deleted": csv_deleted,
            "unverified": {
                "not_compared": summary.unverified.not_compared,
                "host_only": summary.unverified.host_only,
                "deduped": summary.unverified.deduped,
            },
        },
        "items": items,
    });
    pk_cli_core::output::json(&doc);
}

/// Say on stderr which rows were not checked against 1Password. Their
/// passwords may exist only in Google Password Manager.
pub fn print_unverified(u: &Unverified) {
    for note in unverified_notes(u) {
        eprintln!("{note}");
    }
}

fn unverified_notes(u: &Unverified) -> Vec<String> {
    let mut notes = Vec::new();
    if u.not_compared > 0 {
        notes.push(format!(
            "{} row(s) matched an existing login and were not compared. Their passwords are \
             still in Google Password Manager; run with --update to reconcile.",
            u.not_compared
        ));
    }
    if u.host_only > 0 {
        notes.push(format!(
            "{} row(s) had no username and matched the only login on their site on host alone, \
             which may be another account, so that login was not updated, even with --update. \
             Check those logins in 1Password; the rows' passwords are still in Google Password \
             Manager.",
            u.host_only
        ));
    }
    if u.deduped > 0 {
        notes.push(format!(
            "{} row(s) repeated a later row's site and username, and only the later row was \
             synced. The earlier passwords are still in Google Password Manager.",
            u.deduped
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

/// The human line for an export that was deleted.
pub fn deleted_line(csv: &str, u: &Unverified) -> String {
    if u.total() == 0 {
        format!("Deleted the export at {csv}: every credential read from it is in 1Password now.")
    } else {
        format!(
            "Deleted the export at {csv}. Google Password Manager still holds every password in \
             it, including the rows above that were not checked against 1Password."
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

    #[test]
    fn every_unverified_group_gets_its_own_note() {
        assert!(unverified_notes(&Unverified::default()).is_empty());
        let notes = unverified_notes(&Unverified {
            not_compared: 3,
            host_only: 2,
            deduped: 1,
        });
        assert_eq!(notes.len(), 3);
        assert!(notes[0].starts_with("3 row(s) matched an existing login and were not compared"));
        assert!(notes[0].contains("run with --update to reconcile"));
        assert!(notes[1].starts_with("2 row(s) had no username"));
        assert!(notes[2].starts_with("1 row(s) repeated a later row's"));
    }

    #[test]
    fn the_deleted_line_claims_1password_only_when_every_row_was_checked() {
        let all = deleted_line("x.csv", &Unverified::default());
        assert!(all.contains("every credential read from it is in 1Password now"));
        let some = deleted_line(
            "x.csv",
            &Unverified {
                not_compared: 1,
                ..Unverified::default()
            },
        );
        assert!(!some.contains("is in 1Password now"), "{some}");
        assert!(some.contains("Google Password Manager still holds every password"));
    }
}
