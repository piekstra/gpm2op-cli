# gpm2op — internals

Upserts a Google/Chrome Password Manager CSV export into 1Password via the `op`
CLI. Single binary crate. This file orients contributors/agents.

## Module map

- `main.rs` — clap dispatch (`sync`, `check`), exit codes, the "delete your CSV"
  reminder.
- `cli.rs` — clap-derive command/flags.
- `chrome.rs` — parse the export CSV. Columns are located by **header name**
  (case-insensitive), so column order and short rows are tolerated.
- `credential.rs` — `Credential` (normalized row) + host normalization +
  the `op` create JSON template.
- `op.rs` — subprocess wrapper around `op`. Types mirror the subset of
  `op item list/get` JSON we use.
- `sync.rs` — the engine: build an index of existing logins, plan an action per
  credential, execute, and tally a `Summary`.
- `report.rs` — table / JSON / summary rendering.
- `error.rs` — error type, incl. friendly `OpMissing` / `OpNotSignedIn`.

## The one rule that matters: secrets never hit argv

`op` warns that command arguments are visible to other processes. So:

- **Create:** build a JSON item template and pipe it to `op item create -`
  (stdin). See `Credential::create_template` and `Op::create_login`.
- **Update:** `op item get --reveal --format json`, patch only the `PASSWORD`
  field's value in the raw JSON, write to a **`0600` tempfile**
  (`tempfile` crate), `op item edit --template <file>`, then delete. See
  `sync::update_password` and `Op::edit_from_template`.

Never add an assignment-argument (`password=...`) code path.

## Matching & idempotency

- Index existing **Login** items by normalized **host** → list of
  `(username_lowercased, id)`, sourced from `op item list` (its
  `additional_information` field carries the username, and `urls[].href` the
  sites) — so matching needs **no per-item reads**.
- A credential matches when host + username match; same host + different
  username ⇒ a new login. Empty CSV username with exactly one login on the host
  ⇒ match (avoids dupes).
- Default `OnConflict::Skip` = create-missing only (no password reads at all).
  `--update` = `OnConflict::Update`, which reads the existing password and
  writes only if it differs (`unchanged` otherwise). Running twice ⇒ no changes.

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
normalization, template shape, matching/plan decisions, and dedupe — all offline
(no `op`). The live write path can't be unit-tested without an `op` session;
verify it manually with `op item create --dry-run -` and a scratch vault.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```
