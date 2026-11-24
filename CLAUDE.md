# gpm2op — internals

Upserts a Google/Chrome Password Manager CSV export into 1Password via the `op`
CLI. Single binary crate. This file orients contributors/agents.

## Module map

- `main.rs` — clap dispatch (`sync`, `check`), exit codes, and which stream
  each line goes to.
- `cli.rs` — clap-derive command/flags.
- `chrome.rs` — parse the export CSV. Columns are located by **header name**
  (case-insensitive), so column order and short rows are tolerated.
- `credential.rs` — `Credential` (normalized row) + host normalization +
  the `op` create JSON template.
- `op.rs` — subprocess wrapper around `op`. Types mirror the subset of
  `op item list/get` JSON we use.
- `sync.rs` — the engine: build an index of existing logins, plan an action per
  credential, execute, and tally a `Summary`.
- `export.rs` — removes the export after a run with no failed rows, and the
  stderr note for an export left on disk.
- `report.rs` — table / JSON / summary rendering.
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
  `GPM2OP_EDIT_TIMEOUT` seconds (default 120) in any case, failing the row.
  The child inherits the blocked mask on spawn (std does not reset it on
  macOS), so `SignalHold::release_in_child` restores the pre-hold mask in a
  `pre_exec` hook. The mask is per thread: the hold assumes gpm2op is
  single-threaded apart from the pipe readers started under it. Only SIGKILL
  can leave the file.

Never add an assignment-argument (`password=...`) code path.

## The export is deleted after a clean run

The CSV holds every password in plain text, so `sync` removes it once a
non-dry-run finishes with `failed == 0`, also when nothing was created.
Deletion rests on Google Password Manager staying the source: the export is a
copy, so removing it loses nothing. `failed == 0` does not mean every row is
in 1Password, and nothing may claim it does unless `sync::Unverified` is all
zero. It counts, in disjoint groups, the rows not checked against 1Password:
`not_compared` (matched host + username and skipped without reading the
password), `host_only` (empty username matched to the only login on the host,
which may be another account; always skipped, `--update` included) and `deduped` (dropped by
`dedupe`). `--json` reports them as `summary.unverified`; human mode prints a
stderr line per group (`report::print_unverified`), and
`report::deleted_line` says 1Password holds every credential only when the
total is zero. `export::may_delete` decides from the run's end,
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

## Matching & idempotency

- Index existing **Login** items by normalized **host** → list of
  `(username_lowercased, id)`, sourced from `op item list` (its
  `additional_information` field carries the username, and `urls[].href` the
  sites) — so matching needs **no per-item reads**.
- A credential matches when host + username match; same host + different
  username ⇒ a new login. Empty CSV username with exactly one login on the host
  ⇒ a host-only match (avoids dupes). That login may be another account's, so
  `plan` always skips it, even under `--update`, and it counts under
  `Unverified::host_only`.
- Default `OnConflict::Skip` = create-missing only (no password reads at all).
  `--update` = `OnConflict::Update`, which reads the existing password of an
  exact (host + username) match and writes only if it differs (`unchanged`
  otherwise). Running twice ⇒ no changes.

## Scope

Passwords only. TOTP (would decode a Google Authenticator `otpauth-migration`
QR) and passkeys (non-exportable) are intentionally out — see README. If adding
TOTP later, it slots in as a new subcommand + module; the `op` OTP field is set
the same secret-safe way (JSON template with a field of `"type":"OTP"`).

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
