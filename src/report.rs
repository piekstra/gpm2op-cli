//! Human and JSON rendering of sync results.

use crate::sync::{Outcome, OutcomeKind, Summary};
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

pub fn print_json(outcomes: &[Outcome], summary: &Summary) {
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
        "summary": {
            "created": summary.created,
            "updated": summary.updated,
            "unchanged": summary.unchanged,
            "skipped_existing": summary.skipped_existing,
            "failed": summary.failed,
            "planned": summary.planned,
        },
        "items": items,
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
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
