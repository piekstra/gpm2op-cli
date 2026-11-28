# Development

Repo-local facts for working on `gpm2op`. Architecture rationale lives in
[`CLAUDE.md`](../CLAUDE.md); this file is the how-to-develop source of truth.

## Prerequisites

- Rust 1.82+ (`rustup`).
- The 1Password CLI (`op`) 2.x, signed in — needed to run the tool, not to build
  it. On Linux, OpenSSL dev headers (`pkg-config`, `libssl-dev`) for the
  `self-update` HTTP client.

## Layout

Single binary crate. Modules:

```
src/main.rs        # clap dispatch, exit codes
src/cli.rs         # command/flags
src/source.rs      # CSV parsing: Chrome or Apple format by header, row numbers
src/credential.rs  # normalized row + host normalization + op create template
src/op.rs          # op subprocess wrapper (secret-safe)
src/sync.rs        # match / plan / execute / summary
src/export.rs      # removing the export after a run with no failed rows
src/report.rs      # table / JSON / summary rendering (the output layer)
src/selfupdate.rs  # self-update via GitHub releases
tests/sync_export.rs  # end-to-end runs of the binary against a stand-in op
tests/support/bin/op  # the stand-in op (POSIX sh)
tests/fixtures/       # synthetic exports and op's JSON answers
```

## Common tasks

```bash
make build     # cargo build
make test      # cargo test
make lint      # cargo clippy --all-targets -- -D warnings
make fmt       # cargo fmt --all
make check     # fmt-check + lint + test + build (run before pushing)
make install   # cargo install --path .
```

## Invariants (don't regress these)

- **Secrets never on argv.** Creates pipe a JSON template to `op item create -`;
  updates write a `0600` temp file and `op item edit --template`. See
  `op.rs` and `sync::update_password`.
- **Idempotent.** Matching is host + username; running twice makes no second-run
  changes. Default mode never touches existing items.
- **The export does not outlive a clean run.** A non-dry-run with no failed
  rows deletes it unless `--keep-csv`; a failed row keeps it for the rerun; only
  the regular file that was read is removed. See `export.rs`.
- **No claim beyond what was checked.** Rows not checked against 1Password
  (`sync::Unverified`) are counted in `summary.unverified` and named on stderr;
  output says 1Password holds every credential only when there are none.
- **The `--update` temp file never outlives its edit**, short of SIGKILL: stop
  signals are held, forwarded to `op` and re-raised after the file is gone, and
  `op` is killed after `GPM2OP_EDIT_TIMEOUT` seconds (default 120). See
  `op::SignalHold` and `Op::run_held`.
- **A file is read before `op` runs.** Format detection and argument checks
  fail with exit 2 and no `op` call. An unrecognized header is never echoed.
- **Rows are named by number.** Notes and errors give counts and row numbers,
  never a title, site, username or password.
- **`--update` never edits a login holding a passkey** (`sync::has_passkey`).
- **One JSON document.** Under `--json`, stdout carries the DTO alone; progress
  and notes go to stderr.

## Testing

Unit tests are inline and offline (no `op`): CSV parsing, host normalization,
template shape, matching/plan, dedupe, export removal.

`tests/sync_export.rs` runs the binary against `tests/support/bin/op`, a shell
stand-in that answers from `tests/fixtures/op/*.json` and logs argv. A test
selects behavior with environment variables (`FAKE_OP_ITEM_LIST`,
`FAKE_OP_ITEM_GET`, `FAKE_OP_EDIT_FAIL`, `FAKE_OP_EDIT_SIGNAL`, `FAKE_OP_EDIT_SLEEP`; see the
script's header); an export row on `fail.example` stands for a row `op`
refuses. `FAKE_OP_CREATED` collects each create's stdin template so a test
can check the item's fields. These tests are Unix-only.

Export fixtures follow the documented column layouts only (Google's and
Chrome's help, Apple's Passwords export) with invented sites and passwords.
Never copy a row from a real export.

The live write path needs an `op` session; verify manually with
`op item create --dry-run -` against a scratch vault.

## Option: the edit template on stdin

`op item edit <item>` also takes the template on piped stdin (op 2.42.0's
`op item edit --help`: "You can also edit an item using piped input"). Piping
it the way creates already do would remove the temp file, and with it
`SignalHold`, `run_held` and `GPM2OP_EDIT_TIMEOUT`'s reason to exist. It needs
a live test against a scratch vault first, since the temp-file route is the
one proven against real `op`: an item whose password changes, one whose
password is unchanged, a failed edit, and that the resulting item matches the
template. The same help warns that a JSON template edit drops a passkey stored
on the item, by either route.

## Releasing

Version lives in `Cargo.toml` (mirrored in `version.txt`). Tag to release:

```bash
git tag -s -a v0.4.0 -m "gpm2op 0.4.0: …" && git push origin v0.4.0
```

Each version's user-facing changes go in `CHANGELOG.md` with the bump.

`.github/workflows/release.yml` builds the binaries `gpm2op self-update` pulls.
