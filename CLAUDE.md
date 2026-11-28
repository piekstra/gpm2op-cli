# gpm2op — internals

Upserts Google/Chrome Password Manager and Apple Passwords/Safari CSV exports
into 1Password via the `op` CLI. Single binary crate. This file orients contributors/agents.

## Module map

- `main.rs` — clap dispatch (`sync`, `check`), exit codes, and which stream
  each line goes to.
- `cli.rs` — clap-derive command/flags.
- `source.rs` — parse an export CSV. The header decides the `Format`:
  `url,username,password` with no `title` (`name`, `note` optional) is
  Chrome, `Title,URL,Username,Password,Notes,OTPAuth` is Apple; anything
  else, both `name` and `title` included, is refused before `op` runs, and the
  error never repeats the header (a headerless file has a credential there).
  Columns are located by **header name** (case-insensitive), so column order
  and short rows are tolerated. Each row keeps its number (header = row 1).
- `credential.rs` — `Credential` (normalized row, its `Origin` file and row,
  an optional `otpauth://` secret) + host normalization + the `op` create
  JSON template. An empty password gets no password field; an OTP secret
  becomes a `"type":"OTP"` field.
- `op.rs` — subprocess wrapper around `op`. Types mirror the subset of
  `op item list/get` JSON we use.
- `sync.rs` — the engine: build an index of existing logins, plan an action per
  credential, execute, and tally a `Summary`.
- `export.rs` — removes the export after a run with no failed rows, and the
  stderr note for an export left on disk.
- `report.rs` — table / JSON / summary rendering. Notes name rows by number,
  never by title, site, username or password, and name the password manager
  each export came from (`Format::keeper`).
- `error.rs` — error type, incl. friendly `OpMissing` / `OpNotSignedIn`.

## The one rule that matters: secrets never hit argv

`op` warns that command arguments are visible to other processes. So:

- **Create:** build a JSON item template and pipe it to `op item create -`
  (stdin). See `Credential::create_template` and `Op::create_login`.
- **Update:** `op item get --reveal --format json`, patch only the `PASSWORD`
  field's value in the raw JSON, write to a **`0600` tempfile**
  (`tempfile` crate), `op item edit --template <file>`, then delete. See
  `sync::update_password` and `Op::edit_from_template`. `NamedTempFile`
  deletes the file on drop (every return and a panic). `op::SignalHold`
  blocks SIGINT/SIGTERM/SIGHUP/SIGQUIT while the file exists, and its drop
  delivers (or re-raises) a held one after the file is gone. `Op::run_held`
  supervises `op item edit`: it polls for a held signal and forwards it to
  `op`, since a signal sent to gpm2op alone (`kill`, `timeout`, launchd) never
  reaches `op`; `op` gets 5 s to exit before it is killed, and is killed after
  `GPM2OP_EDIT_TIMEOUT` seconds (default 120) in any case, failing the row
  (`parse_edit_timeout`; a rejected value warns once and uses the default).
  The child inherits the blocked mask on spawn (std does not reset it on
  macOS), so `SignalHold::release_in_child` restores the pre-hold mask in a
  `pre_exec` hook. The mask is per thread: the hold assumes gpm2op is
  single-threaded apart from the pipe readers started under it. Only SIGKILL
  can leave the file.

Never add an assignment-argument (`password=...`) code path.

## The export is deleted after a clean run

The CSV holds every password in plain text, so `sync` removes it once a
non-dry-run finishes with `failed == 0`, also when nothing was created.
Deletion rests on Google Password Manager or Apple Passwords staying the source: the export is a
copy, so removing it loses nothing. `failed == 0` does not mean every row is
in 1Password, and nothing may claim it does unless `Summary::complete`:
nothing unverified, and no one-time password or note left out.
`sync::Unverified` counts, in disjoint groups, the rows not checked against 1Password:
`not_compared` (matched host + username and skipped without reading the
password), `host_only` (empty username matched to the only login on the host,
which may be another account; always skipped, `--update` included),
`title_only` (no URL, matched a URL-less login on title + username; always
skipped, `--update` included), `deduped` (dropped by `dedupe`) and `unreadable` (no URL and no username, so
`source::read_csv` dropped them; `main` sets this count). `--json` reports them as `summary.unverified`; human mode prints a
stderr line per group (`report::print_notes`), and
`report::deleted_line` says 1Password holds every credential only when
`Summary::complete`. `export::may_delete` decides from the run's end,
`export::dispose` from the file:

- Kept: `--dry-run`, `--keep-csv`, any failed row (so a rerun can retry; the
  clean rerun deletes it), and an export with no usable rows (it may not be a
  password export at all).
- Only the regular file that was read is removed. `Snapshot` records its length
  and mtime before the read; a symlink, directory, pipe or `/dev/stdin`, or a
  file that changed during the run, is left alone.
- Every kept export except a dry run's gets a stderr note
  (`export::kept_note`), in human and `--json` mode alike. `--json` reports
  `summary.csv_deleted`.

## Output streams

stdout carries results only; progress ("Read N credential(s)…") and notes go to
stderr. Under `--json` stdout is exactly one document (piekstra-cli/1):
`sync-result/v1`, `check-result/v1`, `cli-info/v1`, `self-update/v1`, or the
family error shape.

## Several exports, keep-all and tags

`sync` takes one or more CSVs, read and validated before `preflight`. Their
rows go through one `sync::run`; a failed row keeps every export.

- **Upsert** (`Mode::Upsert`, default): `dedupe` by site key + username, last
  row wins, across files; a password, OTP or note it lacks comes from the
  earlier row.
- **Site key** (`Credential::site_key`, `credential::site_key_for`): the
  normalized host, or `title:<lowercased title>` for a row or vault login with
  no URL, so URL-less logins match by title.
- **Keep-all** (`Mode::KeepAll`, `--keep-all`, conflicts with `--update`):
  `fold_repeats` drops repeats of (site key, lowercased username, password),
  keeping the first.
  `plan` gives `CreateUnlessHeld` with every exact host + username login;
  `PasswordCache` reveals each candidate once and the row is `unchanged` when
  one holds the same password, created otherwise. Host-only matching does not
  apply. Keep-all never edits; the only rows it can leave unverified are
  `unreadable` ones. It reports `repeats` and `conflicting_logins`. A repeat
  without an OTP folds into a kept row of the same key (taking its secret if
  the kept row has none); a different OTP stays a login of its own. Distinct
  notes are kept per row and joined once.
- **Tags** (`--tag`, repeatable, no commas) go to `op item create --tags` on
  argv: tags are not secret.
- **OTP**: `summary.otp` counts secrets added, carried by a row whose login
  exists (`not_added`), and `OTPAuth` values that are not `otpauth://`
  (`ignored`, by row); `notes_not_added` counts notes of rows whose login
  exists. `Summary::complete` (all of these zero and nothing unverified) is
  the only case in which the deletion line says 1Password holds everything.

## Passkeys and `--update`

`op item edit --help` (2.42.0): a JSON template update overwrites a passkey on
the item. 1Password documents no passkey field in `op item get` JSON, and a
user report says `op` shows none, so a passkey cannot be detected reliably.
Hence `--update` needs `--force` outside a dry run (`Error::Confirm`, exit 6,
checked before `op` runs). `update_password` also refuses an item whose
revealed JSON has a key or field type naming a passkey (`has_passkey`, the
shape the desktop app's JSON uses), and that row fails. Passkeys themselves move only by
Apple's Credential Exchange on iPhone/iPad (README, Passkeys).

## Matching & idempotency

- Index existing **Login** items by **site key** (the normalized host, or
  `title:<lowercased title>` for a login with no URL) → list of
  `(username_lowercased, id)`, each pair once, sourced from `op item list` (its
  `additional_information` field carries the username, and `urls[].href` the
  sites) — so matching needs **no per-item reads**, except keep-all's
  comparisons and a row with neither a URL nor a title (below).
- A credential matches when site key + username match. A match on a title key
  is `Match::Title`: like host-only, `plan` always skips it, even under
  `--update` (titles are labels, not sites), and it counts under
  `Unverified::title_only`. A row with no URL and no title from the export
  (`Credential::is_untitled`) gets the key `row:<file>:<row>`, so it never
  matches as a site; `plan` gives it `CreateUnlessHeld` against the URL-less
  `UNTITLED` logins with its username, in every mode, so a rerun adds nothing
  and nothing is updated. Same host + different
  username ⇒ a new login. Empty CSV username with exactly one login on the host
  ⇒ a host-only match (avoids dupes). That login may be another account's, so
  `plan` always skips it, even under `--update`, and it counts under
  `Unverified::host_only`.
- A row with an empty password is never used to update, and is not counted
  as `not_compared` (it has nothing to compare).
- Default `OnConflict::Skip` = create-missing only; it reads no passwords,
  except to compare a row with neither a URL nor a title against existing
  URL-less "Untitled Login" items of the same username.
  `--update` = `OnConflict::Update`, which reads the existing password of an
  exact (host + username) match and writes only if it differs (`unchanged`
  otherwise). Running twice ⇒ no changes.

## Scope

Passwords, plus the one-time-password secrets Apple's export carries. Google
Authenticator seeds (an `otpauth-migration` QR) and passkeys (no file export)
are out — see README.

## Self-update & releases

`src/selfupdate.rs` (`gpm2op self-update`) uses the `self_update` crate's GitHub
backend to pull the platform binary from this repo's Releases and swap it in
place. Assets are matched by Rust target triple, so the release workflow
(`.github/workflows/release.yml`, triggered by a `v*` tag) names them
`gpm2op-<tag>-<target>.tar.gz`. `version.txt` mirrors the Cargo version.

## Testing

Unit tests cover CSV parsing (incl. short/missing-note rows), host
normalization, template shape, matching/plan decisions, dedupe, and the export
removal rules — all offline (no `op`).

`tests/sync_export.rs` runs the built binary end to end against
`tests/support/bin/op`, a shell stand-in for `op` that answers from
`tests/fixtures/op/*.json` and logs each call's argv. PATH holds only that
directory and the system's, so no test can reach the real `op` or raise a
1Password prompt. They cover deleting and keeping the export, the unverified
counts and their notes, one JSON document on stdout, the `--update` template's
`0600` mode and removal (on success, a failed edit, SIGINT/SIGTERM mid-edit, a
SIGTERM sent to gpm2op alone while `op` sleeps, and an edit past
`GPM2OP_EDIT_TIMEOUT`), and that no password reaches argv.
Export fixtures are synthetic CSVs in `tests/fixtures/`.

The real `op` write path still needs a manual check against a scratch vault
(`op item create --dry-run -`).

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```
