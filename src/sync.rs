//! The upsert engine: match CSV credentials against existing 1Password logins,
//! then create the missing ones (and, with `--update`, reconcile changed
//! passwords).

use crate::credential::{host_of, Credential};
use crate::error::{Error, Result};
use crate::op::{Op, OpItem};
use std::collections::HashMap;

/// What to do about a credential that already exists in 1Password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// Leave existing items untouched (default — pure catch-up).
    Skip,
    /// Update the stored password when it differs from the CSV.
    Update,
}

/// The decided action for one credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create,
    UpdatePassword { id: String },
    SkipExisting { id: String },
}

/// Per-credential outcome for reporting.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub title: String,
    pub host: String,
    pub username: String,
    pub result: OutcomeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutcomeKind {
    Created,
    Updated,
    Unchanged,
    SkippedExisting,
    Failed(String),
    Planned(&'static str),
}

/// Aggregate counts.
#[derive(Debug, Default, Clone)]
pub struct Summary {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub skipped_existing: usize,
    pub failed: usize,
    pub planned: usize,
}

/// Index of existing login items keyed by normalized host → (username_lc, id).
struct Index {
    by_host: HashMap<String, Vec<(String, String)>>,
}

impl Index {
    fn build(items: &[OpItem]) -> Index {
        let mut by_host: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for it in items {
            let username = it.username().unwrap_or("").to_lowercase();
            for u in &it.urls {
                if let Some(host) = host_of(&u.href) {
                    by_host
                        .entry(host)
                        .or_default()
                        .push((username.clone(), it.id.clone()));
                }
            }
        }
        Index { by_host }
    }

    /// Find an existing item id matching this credential's host+username.
    fn find(&self, cred: &Credential) -> Option<String> {
        let host = cred.host()?;
        let candidates = self.by_host.get(&host)?;
        let want = cred.username.to_lowercase();
        // Prefer an exact username match on the same host.
        if let Some((_, id)) = candidates.iter().find(|(u, _)| *u == want) {
            return Some(id.clone());
        }
        // If the CSV row has no username and exactly one login exists for the
        // host, treat that as the match (avoids a duplicate).
        if want.is_empty() && candidates.len() == 1 {
            return Some(candidates[0].1.clone());
        }
        None
    }
}

/// De-duplicate credentials by (host, username), keeping the last occurrence.
pub fn dedupe(creds: Vec<Credential>) -> Vec<Credential> {
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut out: Vec<Credential> = Vec::new();
    for c in creds {
        let key = c.identity();
        if let Some(&idx) = seen.get(&key) {
            out[idx] = c;
        } else {
            seen.insert(key, out.len());
            out.push(c);
        }
    }
    out
}

/// Decide the action for each credential given the existing index.
fn plan(creds: &[Credential], index: &Index, on_conflict: OnConflict) -> Vec<(Credential, Action)> {
    creds
        .iter()
        .map(|c| {
            let action = match index.find(c) {
                None => Action::Create,
                Some(id) => match on_conflict {
                    OnConflict::Skip => Action::SkipExisting { id },
                    OnConflict::Update => Action::UpdatePassword { id },
                },
            };
            (c.clone(), action)
        })
        .collect()
}

/// Run the sync end-to-end.
pub fn run(
    op: &Op,
    vault: &str,
    creds: Vec<Credential>,
    on_conflict: OnConflict,
    dry_run: bool,
) -> Result<(Vec<Outcome>, Summary)> {
    let existing = op.list_logins(vault)?;
    let index = Index::build(&existing);
    let creds = dedupe(creds);
    let planned = plan(&creds, &index, on_conflict);

    let mut outcomes = Vec::with_capacity(planned.len());
    let mut summary = Summary::default();

    for (cred, action) in planned {
        let host = cred.host().unwrap_or_default();
        let kind = if dry_run {
            summary.planned += 1;
            match action {
                Action::Create => OutcomeKind::Planned("create"),
                Action::UpdatePassword { .. } => OutcomeKind::Planned("update"),
                Action::SkipExisting { .. } => OutcomeKind::Planned("skip"),
            }
        } else {
            execute(op, vault, &cred, &action, &mut summary)
        };
        outcomes.push(Outcome {
            title: cred.title.clone(),
            host,
            username: cred.username.clone(),
            result: kind,
        });
    }
    Ok((outcomes, summary))
}

fn execute(
    op: &Op,
    vault: &str,
    cred: &Credential,
    action: &Action,
    summary: &mut Summary,
) -> OutcomeKind {
    match action {
        Action::Create => match op.create_login(vault, &cred.create_template()) {
            Ok(_) => {
                summary.created += 1;
                OutcomeKind::Created
            }
            Err(e) => {
                summary.failed += 1;
                OutcomeKind::Failed(short(&e))
            }
        },
        Action::SkipExisting { .. } => {
            summary.skipped_existing += 1;
            OutcomeKind::SkippedExisting
        }
        Action::UpdatePassword { id } => match update_password(op, id, cred) {
            Ok(true) => {
                summary.updated += 1;
                OutcomeKind::Updated
            }
            Ok(false) => {
                summary.unchanged += 1;
                OutcomeKind::Unchanged
            }
            Err(e) => {
                summary.failed += 1;
                OutcomeKind::Failed(short(&e))
            }
        },
    }
}

/// Update an existing item's password only if it actually differs. Returns
/// whether a change was written.
fn update_password(op: &Op, id: &str, cred: &Credential) -> Result<bool> {
    let mut item = op.get_revealed_value(id)?;
    let fields = item
        .get_mut("fields")
        .and_then(|f| f.as_array_mut())
        .ok_or_else(|| Error::Parse("item has no fields array".into()))?;

    let mut changed = false;
    for f in fields.iter_mut() {
        if f.get("purpose").and_then(|p| p.as_str()) == Some("PASSWORD") {
            let current = f.get("value").and_then(|v| v.as_str()).unwrap_or("");
            if current == cred.password {
                return Ok(false); // identical — nothing to do
            }
            f["value"] = serde_json::Value::String(cred.password.clone());
            changed = true;
        }
    }
    if !changed {
        return Ok(false);
    }
    op.edit_from_template(id, &item.to_string())?;
    Ok(true)
}

fn short(e: &Error) -> String {
    e.to_string().lines().next().unwrap_or("error").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::{OpItem, OpUrl};

    fn login(id: &str, host: &str, user: &str) -> OpItem {
        OpItem {
            id: id.into(),
            additional_information: Some(user.into()),
            urls: vec![OpUrl {
                href: format!("https://{host}"),
            }],
            fields: vec![],
        }
    }

    fn cred(host: &str, user: &str, pw: &str) -> Credential {
        Credential {
            title: host.into(),
            url: format!("https://{host}"),
            username: user.into(),
            password: pw.into(),
            note: "".into(),
        }
    }

    #[test]
    fn creates_when_absent_skips_when_present() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let creds = vec![
            cred("github.com", "jane", "pw"), // exists
            cred("gitlab.com", "jane", "pw"), // new
        ];
        let planned = plan(&creds, &index, OnConflict::Skip);
        assert!(matches!(planned[0].1, Action::SkipExisting { .. }));
        assert_eq!(planned[1].1, Action::Create);
    }

    #[test]
    fn same_host_different_user_creates() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let creds = vec![cred("github.com", "bob", "pw")];
        let planned = plan(&creds, &index, OnConflict::Skip);
        assert_eq!(planned[0].1, Action::Create);
    }

    #[test]
    fn update_mode_targets_existing() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let creds = vec![cred("github.com", "jane", "newpw")];
        let planned = plan(&creds, &index, OnConflict::Update);
        assert_eq!(planned[0].1, Action::UpdatePassword { id: "id1".into() });
    }

    #[test]
    fn dedupe_keeps_last() {
        let creds = vec![
            cred("x.com", "u", "old"),
            cred("x.com", "u", "new"),
            cred("y.com", "u", "z"),
        ];
        let out = dedupe(creds);
        assert_eq!(out.len(), 2);
        let x = out
            .iter()
            .find(|c| c.host().as_deref() == Some("x.com"))
            .unwrap();
        assert_eq!(x.password, "new");
    }
}
