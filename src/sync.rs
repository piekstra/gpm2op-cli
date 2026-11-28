//! The upsert engine: match CSV credentials against existing 1Password logins,
//! then create the missing ones (and, with `--update`, reconcile changed
//! passwords). Keep-all mode instead creates a login for every distinct site,
//! username and password, so conflicting exports lose nothing.

use crate::credential::{host_of, site_key_for, Credential, Origin};
use crate::error::{Error, Result};
use crate::op::{Op, OpItem};
use std::collections::{HashMap, HashSet};

/// What to do about a credential that already exists in 1Password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// Leave existing items untouched (default — pure catch-up).
    Skip,
    /// Update the stored password when it differs from the CSV.
    Update,
}

/// How rows become logins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// One login per site and username; the last row wins.
    Upsert(OnConflict),
    /// One login per distinct site, username and password. A login already in
    /// the vault with the same site, username and password is not created
    /// again.
    KeepAll,
}

impl Mode {
    pub fn id(self) -> &'static str {
        match self {
            Mode::Upsert(_) => "upsert",
            Mode::KeepAll => "keep-all",
        }
    }
}

/// The settings of one run.
pub struct Options<'a> {
    pub vault: &'a str,
    pub mode: Mode,
    pub dry_run: bool,
    /// Tags put on every login the run creates.
    pub tags: &'a [String],
}

/// The decided action for one credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create,
    UpdatePassword {
        id: String,
    },
    SkipExisting {
        id: String,
    },
    /// Keep-all: create unless one of these logins (same site and username)
    /// already holds the same password.
    CreateUnlessHeld {
        candidates: Vec<String>,
    },
}

/// Per-credential outcome for reporting.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub origin: Origin,
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
    /// Keep-all: rows that repeated an earlier row's site, username, password
    /// and one-time password exactly, across every file of the run. Their
    /// notes, when different, were added to the earlier row's.
    pub repeats: usize,
    /// Keep-all: site and username pairs that the run's exports give more
    /// than one password for. Each password became its own login.
    pub conflicting_logins: usize,
    pub otp: OtpCounts,
    /// Rows with a note whose login already existed; the note was not added.
    pub notes_not_added: usize,
    pub unverified: Unverified,
}

impl Summary {
    /// Whether everything the exports carried reached 1Password: every row
    /// checked, and no one-time password or note left out.
    pub fn complete(&self) -> bool {
        self.unverified.total() == 0
            && self.otp.not_added == 0
            && self.otp.ignored.is_empty()
            && self.notes_not_added == 0
    }
}

/// What happened to the one-time-password secrets in Apple's `OTPAuth` column.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OtpCounts {
    /// Written as a one-time-password field on a login this run created (or,
    /// in a dry run, would create).
    pub added: usize,
    /// Carried by a row whose login already exists; the secret was not added
    /// to it.
    pub not_added: usize,
    /// Row numbers, per file, of values that are not an `otpauth://` URI and
    /// were not imported. Set by the caller, which reads the files.
    pub ignored: Vec<Origin>,
}

/// Rows whose password this run did not check against 1Password, in five
/// disjoint groups. Their passwords may exist only in the password manager
/// the export came from. A dry run counts the rows it would leave unverified.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Unverified {
    /// Matched an existing login on host and username and were left as-is
    /// without reading its password (default mode). A row with no password
    /// has nothing to compare and is not counted.
    pub not_compared: usize,
    /// Had no username and matched the only login on their host, which may
    /// be another account. That login is always left untouched, `--update`
    /// included.
    pub host_only: usize,
    /// Had no URL and matched a login with no URL on title and username,
    /// which may be another device's or account's. That login is always left
    /// untouched, `--update` included.
    pub title_only: usize,
    /// Repeated a later row's host and username; only the last was synced.
    pub deduped: usize,
    /// Had neither a URL nor a username, so `source::read_csv` dropped them
    /// before the sync. Set by the caller, which holds that count.
    pub unreadable: usize,
}

impl Unverified {
    pub fn total(&self) -> usize {
        self.not_compared + self.host_only + self.title_only + self.deduped + self.unreadable
    }
}

/// How a credential matched an existing login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Match {
    Exact,
    HostOnly,
    /// Same title and username, both with no URL. Titles are labels, not
    /// sites, so this is never enough to update a login.
    Title,
}

/// Index of existing login items keyed by site key (normalized host, or the
/// title of a login with no URL) → (username_lc, id).
struct Index {
    by_host: HashMap<String, Vec<(String, String)>>,
}

impl Index {
    fn build(items: &[OpItem]) -> Index {
        let mut by_host: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for it in items {
            let username = it.username().unwrap_or("").to_lowercase();
            let mut keys: Vec<String> = it.urls.iter().filter_map(|u| host_of(&u.href)).collect();
            if keys.is_empty() {
                keys.push(site_key_for(None, &it.title));
            }
            for key in keys {
                let entry = by_host.entry(key).or_default();
                let pair = (username.clone(), it.id.clone());
                if !entry.contains(&pair) {
                    entry.push(pair);
                }
            }
        }
        Index { by_host }
    }

    /// Find an existing item id matching this credential's site+username.
    fn find(&self, cred: &Credential) -> Option<(String, Match)> {
        let key = cred.site_key();
        let candidates = self.by_host.get(&key)?;
        let want = cred.username.to_lowercase();
        let by_title = key.starts_with("title:");
        // Prefer an exact username match on the same host.
        if let Some((_, id)) = candidates.iter().find(|(u, _)| *u == want) {
            let how = if by_title { Match::Title } else { Match::Exact };
            return Some((id.clone(), how));
        }
        if by_title {
            return None;
        }
        // If the CSV row has no username and exactly one login exists for the
        // host, treat that as the match (avoids a duplicate).
        if want.is_empty() && candidates.len() == 1 {
            return Some((candidates[0].1.clone(), Match::HostOnly));
        }
        None
    }

    /// Every existing login on this credential's site with its username.
    fn exact(&self, cred: &Credential) -> Vec<String> {
        let want = cred.username.to_lowercase();
        self.by_host
            .get(&cred.site_key())
            .map(|c| {
                c.iter()
                    .filter(|(u, _)| *u == want)
                    .map(|(_, id)| id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// De-duplicate credentials by (site, username). The last row wins, but a
/// field it leaves empty (password, one-time password, note) keeps the
/// earlier row's value, so a later Chrome row does not drop an Apple row's
/// code and a passwordless row does not blank a password.
pub fn dedupe(creds: Vec<Credential>) -> Vec<Credential> {
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut out: Vec<Credential> = Vec::new();
    for mut c in creds {
        let key = c.identity();
        if let Some(&idx) = seen.get(&key) {
            let earlier = &out[idx];
            if c.password.is_empty() {
                c.password = earlier.password.clone();
            }
            if c.otp.is_none() {
                c.otp = earlier.otp.clone();
            }
            if c.note.is_empty() {
                c.note = earlier.note.clone();
            }
            out[idx] = c;
        } else {
            seen.insert(key, out.len());
            out.push(c);
        }
    }
    out
}

/// Keep-all: drop exact repeats (same site, username and password), keeping
/// the first row. A repeat's one-time password fills a kept row that has
/// none; a different one keeps the repeat as a login of its own. Every
/// distinct note of the folded rows is kept, in order. Returns the rows kept
/// and the number dropped.
pub fn fold_repeats(creds: Vec<Credential>) -> (Vec<Credential>, usize) {
    let mut groups: HashMap<(String, String, String), Vec<usize>> = HashMap::new();
    let mut out: Vec<Credential> = Vec::new();
    let mut notes: Vec<Vec<String>> = Vec::new();
    let mut repeats = 0;
    for c in creds {
        let group = groups.entry(c.secret_identity()).or_default();
        let fits = group.iter().copied().find(|&i| {
            let kept = &out[i].otp;
            kept.is_none() || c.otp.is_none() || *kept == c.otp
        });
        match fits {
            Some(i) => {
                repeats += 1;
                if out[i].otp.is_none() {
                    out[i].otp = c.otp;
                }
                if !c.note.is_empty() && !notes[i].contains(&c.note) {
                    notes[i].push(c.note);
                }
            }
            None => {
                group.push(out.len());
                notes.push(if c.note.is_empty() {
                    Vec::new()
                } else {
                    vec![c.note.clone()]
                });
                out.push(c);
            }
        }
    }
    for (c, n) in out.iter_mut().zip(notes) {
        c.note = n.join("\n\n");
    }
    (out, repeats)
}

/// Keep-all: how many (site, username) pairs carry more than one password.
fn conflicting_logins(creds: &[Credential]) -> usize {
    let mut passwords: HashMap<(String, String), HashSet<&str>> = HashMap::new();
    for c in creds {
        passwords
            .entry(c.identity())
            .or_default()
            .insert(c.password.as_str());
    }
    passwords.values().filter(|p| p.len() > 1).count()
}

/// One credential's decided action, and how it matched.
struct Step {
    cred: Credential,
    action: Action,
    matched: Option<Match>,
    mode: Mode,
}

/// Decide the action for each credential given the existing index.
fn plan(creds: &[Credential], index: &Index, mode: Mode) -> Vec<Step> {
    creds
        .iter()
        .map(|c| {
            let (action, matched) = match mode {
                Mode::KeepAll => {
                    let candidates = index.exact(c);
                    if candidates.is_empty() {
                        (Action::Create, None)
                    } else {
                        (Action::CreateUnlessHeld { candidates }, Some(Match::Exact))
                    }
                }
                Mode::Upsert(on_conflict) => {
                    let found = index.find(c);
                    let matched = found.as_ref().map(|(_, m)| *m);
                    let action = match found {
                        None => Action::Create,
                        // A host-only match may be another account's login, so
                        // even --update must not overwrite its password.
                        Some((id, Match::HostOnly)) => Action::SkipExisting { id },
                        // Neither may a match on title alone.
                        Some((id, Match::Title)) => Action::SkipExisting { id },
                        // A row with no password has nothing to write; --update
                        // must not blank the stored one.
                        Some((id, Match::Exact)) if c.password.is_empty() => {
                            Action::SkipExisting { id }
                        }
                        Some((id, Match::Exact)) => match on_conflict {
                            OnConflict::Skip => Action::SkipExisting { id },
                            OnConflict::Update => Action::UpdatePassword { id },
                        },
                    };
                    (action, matched)
                }
            };
            Step {
                cred: c.clone(),
                action,
                matched,
                mode,
            }
        })
        .collect()
}

/// Add a row that did not fail to the unverified counts it belongs to.
fn tally_unverified(unverified: &mut Unverified, step: &Step) {
    if step.mode == Mode::KeepAll {
        return;
    }
    match (step.matched, &step.action) {
        (Some(Match::HostOnly), _) => unverified.host_only += 1,
        (Some(Match::Title), _) if !step.cred.password.is_empty() => unverified.title_only += 1,
        (_, Action::SkipExisting { .. }) if !step.cred.password.is_empty() => {
            unverified.not_compared += 1
        }
        _ => {}
    }
}

/// Count what became of a row's one-time-password secret and note.
fn tally_extras(summary: &mut Summary, cred: &Credential, kind: &OutcomeKind) {
    let created = match kind {
        OutcomeKind::Created | OutcomeKind::Planned("create") => true,
        OutcomeKind::Failed(_) => return,
        _ => false,
    };
    if cred.otp.is_some() {
        if created {
            summary.otp.added += 1;
        } else {
            summary.otp.not_added += 1;
        }
    }
    if !cred.note.is_empty() && !created {
        summary.notes_not_added += 1;
    }
}

/// Run the sync end-to-end.
pub fn run(op: &Op, opts: &Options, creds: Vec<Credential>) -> Result<(Vec<Outcome>, Summary)> {
    let existing = op.list_logins(opts.vault)?;
    let index = Index::build(&existing);
    let read = creds.len();
    let mut summary = Summary::default();
    let creds = match opts.mode {
        Mode::Upsert(_) => {
            let creds = dedupe(creds);
            summary.unverified.deduped = read - creds.len();
            creds
        }
        Mode::KeepAll => {
            let (creds, repeats) = fold_repeats(creds);
            summary.repeats = repeats;
            summary.conflicting_logins = conflicting_logins(&creds);
            creds
        }
    };
    let planned = plan(&creds, &index, opts.mode);

    let mut outcomes = Vec::with_capacity(planned.len());
    let mut passwords = PasswordCache::default();

    for step in planned {
        let Step { cred, action, .. } = &step;
        let host = cred.host().unwrap_or_default();
        let kind = if opts.dry_run {
            let kind = match action {
                Action::Create => OutcomeKind::Planned("create"),
                Action::UpdatePassword { .. } => OutcomeKind::Planned("update"),
                Action::SkipExisting { .. } => OutcomeKind::Planned("skip"),
                Action::CreateUnlessHeld { candidates } => {
                    match passwords.held(op, candidates, &cred.password) {
                        Ok(true) => OutcomeKind::Planned("skip"),
                        Ok(false) => OutcomeKind::Planned("create"),
                        Err(e) => {
                            summary.failed += 1;
                            OutcomeKind::Failed(short(&e))
                        }
                    }
                }
            };
            if matches!(kind, OutcomeKind::Planned(_)) {
                summary.planned += 1;
            }
            kind
        } else {
            execute(op, opts, cred, action, &mut passwords, &mut summary)
        };
        if !matches!(kind, OutcomeKind::Failed(_)) {
            tally_unverified(&mut summary.unverified, &step);
        }
        tally_extras(&mut summary, cred, &kind);
        outcomes.push(Outcome {
            origin: cred.origin,
            title: cred.title.clone(),
            host,
            username: cred.username.clone(),
            result: kind,
        });
    }
    Ok((outcomes, summary))
}

/// Keep-all reads each existing login's password at most once per run.
#[derive(Default)]
struct PasswordCache {
    by_id: HashMap<String, String>,
}

impl PasswordCache {
    /// Whether any of `ids` holds `password`. A row with no password matches a
    /// login with no password.
    fn held(&mut self, op: &Op, ids: &[String], password: &str) -> Result<bool> {
        for id in ids {
            if !self.by_id.contains_key(id) {
                let item = op.get_revealed_value(id)?;
                self.by_id.insert(id.clone(), password_of(&item));
            }
            if self.by_id[id] == password {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Whether an item's JSON shows a passkey anywhere: a key or a field type
/// naming one. Erring towards yes only costs an edit that is left undone.
fn has_passkey(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(k, v)| {
            k.to_ascii_lowercase().contains("passkey")
                || (k == "type"
                    && v.as_str()
                        .is_some_and(|t| t.eq_ignore_ascii_case("PASSKEY")))
                || has_passkey(v)
        }),
        serde_json::Value::Array(items) => items.iter().any(has_passkey),
        _ => false,
    }
}

/// The value of a revealed item's password field, or empty.
fn password_of(item: &serde_json::Value) -> String {
    item.get("fields")
        .and_then(|f| f.as_array())
        .and_then(|fields| {
            fields
                .iter()
                .find(|f| f.get("purpose").and_then(|p| p.as_str()) == Some("PASSWORD"))
        })
        .and_then(|f| f.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn execute(
    op: &Op,
    opts: &Options,
    cred: &Credential,
    action: &Action,
    passwords: &mut PasswordCache,
    summary: &mut Summary,
) -> OutcomeKind {
    let create = |summary: &mut Summary| match op.create_login(
        opts.vault,
        &cred.create_template(),
        opts.tags,
    ) {
        Ok(_) => {
            summary.created += 1;
            OutcomeKind::Created
        }
        Err(e) => {
            summary.failed += 1;
            OutcomeKind::Failed(short(&e))
        }
    };
    match action {
        Action::Create => create(summary),
        Action::CreateUnlessHeld { candidates } => {
            match passwords.held(op, candidates, &cred.password) {
                Ok(true) => {
                    summary.unchanged += 1;
                    OutcomeKind::Unchanged
                }
                Ok(false) => create(summary),
                Err(e) => {
                    summary.failed += 1;
                    OutcomeKind::Failed(short(&e))
                }
            }
        }
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
    if password_of(&item) == cred.password {
        return Ok(false);
    }
    // op 2.42.0's `op item edit --help`: "If you use a JSON template to
    // update an item that contains a passkey, the passkey will be
    // overwritten."
    if has_passkey(&item) {
        return Err(Error::Op(
            "this login holds a passkey, which a template edit would overwrite; change its \
             password in the 1Password app"
                .into(),
        ));
    }
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

/// One line for a failed row. `op`'s own message is the cause, and its
/// `Display` puts that message on the line after a header.
fn short(e: &Error) -> String {
    let text = match e {
        Error::Op(m) => m.clone(),
        other => other.to_string(),
    };
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("error")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::{OpItem, OpUrl};

    fn login(id: &str, host: &str, user: &str) -> OpItem {
        OpItem {
            id: id.into(),
            title: String::new(),
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
            ..Credential::default()
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
        let planned = plan(&creds, &index, Mode::Upsert(OnConflict::Skip));
        assert!(matches!(planned[0].action, Action::SkipExisting { .. }));
        assert_eq!(planned[1].action, Action::Create);
    }

    #[test]
    fn same_host_different_user_creates() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let creds = vec![cred("github.com", "bob", "pw")];
        let planned = plan(&creds, &index, Mode::Upsert(OnConflict::Skip));
        assert_eq!(planned[0].action, Action::Create);
    }

    #[test]
    fn update_mode_targets_existing() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let creds = vec![cred("github.com", "jane", "newpw")];
        let planned = plan(&creds, &index, Mode::Upsert(OnConflict::Update));
        assert_eq!(
            planned[0].action,
            Action::UpdatePassword { id: "id1".into() }
        );
    }

    #[test]
    fn an_empty_username_matches_the_only_login_on_its_host() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let planned = plan(
            &[cred("github.com", "", "pw")],
            &index,
            Mode::Upsert(OnConflict::Skip),
        );
        assert_eq!(planned[0].matched, Some(Match::HostOnly));
        let mut unverified = Unverified::default();
        tally_unverified(&mut unverified, &planned[0]);
        assert_eq!(
            unverified,
            Unverified {
                host_only: 1,
                ..Unverified::default()
            }
        );
    }

    #[test]
    fn update_never_overwrites_a_login_matched_on_host_alone() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let planned = plan(
            &[cred("github.com", "", "pw")],
            &index,
            Mode::Upsert(OnConflict::Update),
        );
        assert_eq!(planned[0].action, Action::SkipExisting { id: "id1".into() });
        assert_eq!(planned[0].matched, Some(Match::HostOnly));
    }

    #[test]
    fn unverified_groups_are_disjoint() {
        let existing = vec![
            login("id1", "github.com", "jane"),
            login("id2", "gitlab.com", "jane"),
        ];
        let index = Index::build(&existing);
        let creds = vec![
            cred("github.com", "jane", "pw"),  // exact match, not compared
            cred("gitlab.com", "", "pw"),      // host-only match
            cred("example.com", "jane", "pw"), // created: verified
        ];
        for (mode, want_not_compared) in [(OnConflict::Skip, 1), (OnConflict::Update, 0)] {
            let mut unverified = Unverified::default();
            for step in plan(&creds, &index, Mode::Upsert(mode)) {
                tally_unverified(&mut unverified, &step);
            }
            assert_eq!(unverified.not_compared, want_not_compared, "{mode:?}");
            assert_eq!(unverified.host_only, 1, "{mode:?}");
        }
    }

    #[test]
    fn a_failed_row_names_ops_message() {
        let e = Error::Op("[ERROR] 2026/01/01 00:00:00 vault is read-only\nmore".into());
        assert_eq!(short(&e), "[ERROR] 2026/01/01 00:00:00 vault is read-only");
        assert_eq!(
            short(&Error::Parse("bad json".into())),
            "could not parse `op` output: bad json"
        );
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

    #[test]
    fn fold_repeats_keeps_the_first_and_every_distinct_note() {
        let mut a = cred("x.com", "u", "pw");
        a.note = "one".into();
        let mut b = cred("X.com", "U", "pw");
        b.note = "two".into();
        let mut c = cred("x.com", "u", "pw");
        c.note = "one".into();
        let other = cred("x.com", "u", "other");
        let (out, repeats) = fold_repeats(vec![a, b, c, other]);
        assert_eq!(repeats, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].note, "one\n\ntwo");
        assert_eq!(conflicting_logins(&out), 1);
    }

    #[test]
    fn a_missing_one_time_password_folds_and_a_different_one_does_not() {
        let with = |otp: &str| {
            let mut c = cred("x.com", "u", "pw");
            c.otp = Some(otp.into());
            c
        };
        let (out, repeats) = fold_repeats(vec![
            cred("x.com", "u", "pw"),
            with("otpauth://totp/a?secret=AAAA"),
            with("otpauth://totp/b?secret=BBBB"),
        ]);
        assert_eq!((out.len(), repeats), (2, 1));
        assert_eq!(out[0].otp.as_deref(), Some("otpauth://totp/a?secret=AAAA"));
        assert_eq!(out[1].otp.as_deref(), Some("otpauth://totp/b?secret=BBBB"));
    }

    #[test]
    fn a_repeated_note_with_a_blank_line_is_kept_once() {
        let mut a = cred("x.com", "u", "pw");
        a.note = "first\n\nsecond".into();
        let b = a.clone();
        let (out, _) = fold_repeats(vec![a, b]);
        assert_eq!(out[0].note, "first\n\nsecond");
    }

    #[test]
    fn dedupe_fills_what_the_later_row_lacks() {
        let mut apple = cred("x.com", "u", "pw");
        apple.otp = Some("otpauth://totp/a?secret=AAAA".into());
        apple.note = "n".into();
        let chrome = cred("x.com", "u", "");
        let out = dedupe(vec![apple, chrome]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].password, "pw");
        assert!(out[0].otp.is_some());
        assert_eq!(out[0].note, "n");
    }

    #[test]
    fn a_title_match_is_never_updated() {
        let mut item = login("id1", "unused", "admin");
        item.urls.clear();
        item.title = "Router".into();
        let index = Index::build(&[item]);
        let mut c = cred("unused", "admin", "pw");
        c.url.clear();
        c.title = "Router".into();
        c.titled = true;
        let planned = plan(&[c], &index, Mode::Upsert(OnConflict::Update));
        assert_eq!(planned[0].action, Action::SkipExisting { id: "id1".into() });
        assert_eq!(planned[0].matched, Some(Match::Title));
        let mut unverified = Unverified::default();
        tally_unverified(&mut unverified, &planned[0]);
        assert_eq!(unverified.title_only, 1);
        assert_eq!(unverified.not_compared, 0);
    }

    #[test]
    fn a_row_with_no_url_matches_a_vault_login_with_no_url_by_title() {
        let mut item = login("id1", "unused", "admin");
        item.urls.clear();
        item.title = "Router".into();
        let index = Index::build(&[item]);
        let mut c = cred("unused", "admin", "pw");
        c.url.clear();
        c.title = "router".into();
        c.titled = true;
        assert_eq!(index.exact(&c), vec!["id1".to_string()]);
        let mut other = c.clone();
        other.title = "NAS".into();
        assert!(index.exact(&other).is_empty());
        let (out, repeats) = fold_repeats(vec![c, other]);
        assert_eq!((out.len(), repeats), (2, 0));
    }

    #[test]
    fn keep_all_compares_only_exact_matches_and_never_updates() {
        let existing = vec![
            login("id1", "github.com", "jane"),
            login("id2", "gitlab.com", "jane"),
        ];
        let index = Index::build(&existing);
        let creds = vec![
            cred("github.com", "jane", "pw"),
            // An empty username is not matched on host alone in keep-all.
            cred("gitlab.com", "", "pw"),
        ];
        let planned = plan(&creds, &index, Mode::KeepAll);
        assert_eq!(
            planned[0].action,
            Action::CreateUnlessHeld {
                candidates: vec!["id1".into()]
            }
        );
        assert_eq!(planned[1].action, Action::Create);
        let mut unverified = Unverified::default();
        for step in &planned {
            tally_unverified(&mut unverified, step);
        }
        assert_eq!(unverified, Unverified::default());
    }

    #[test]
    fn update_skips_a_row_with_no_password() {
        let existing = vec![login("id1", "github.com", "jane")];
        let index = Index::build(&existing);
        let planned = plan(
            &[cred("github.com", "jane", "")],
            &index,
            Mode::Upsert(OnConflict::Update),
        );
        assert_eq!(planned[0].action, Action::SkipExisting { id: "id1".into() });
        let mut unverified = Unverified::default();
        tally_unverified(&mut unverified, &planned[0]);
        assert_eq!(unverified.not_compared, 0);
    }

    #[test]
    fn a_passkey_anywhere_in_the_item_is_found() {
        assert!(!has_passkey(&serde_json::json!({"fields": [
            {"type": "CONCEALED", "purpose": "PASSWORD", "value": "pw"}
        ]})));
        assert!(has_passkey(
            &serde_json::json!({"fields": [{"type": "PASSKEY"}]})
        ));
        assert!(has_passkey(
            &serde_json::json!({"passkey": {"rp_id": "example.com"}})
        ));
    }

    #[test]
    fn the_password_of_a_revealed_item() {
        let item = serde_json::json!({"fields": [
            {"purpose": "USERNAME", "value": "jane"},
            {"purpose": "PASSWORD", "value": "pw"},
        ]});
        assert_eq!(password_of(&item), "pw");
        assert_eq!(password_of(&serde_json::json!({"fields": []})), "");
    }
}
