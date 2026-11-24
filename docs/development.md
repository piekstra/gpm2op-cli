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
src/chrome.rs      # CSV parsing (header-indexed, tolerant)
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
- **One JSON document.** Under `--json`, stdout carries the DTO alone; progress
  and notes go to stderr.

## Testing

Unit tests are inline and offline (no `op`): CSV parsing, host normalization,
template shape, matching/plan, dedupe, export removal.

`tests/sync_export.rs` runs the binary against `tests/support/bin/op`, a shell
stand-in that answers from `tests/fixtures/op/*.json` and logs argv. A test
selects behavior with environment variables (`FAKE_OP_ITEM_LIST`,
`FAKE_OP_EDIT_FAIL`, `FAKE_OP_EDIT_SIGNAL`; see the script's header); an export
row on `fail.example` stands for a row `op` refuses. These tests are Unix-only.

The live write path needs an `op` session; verify manually with
`op item create --dry-run -` against a scratch vault.

## Releasing

Version lives in `Cargo.toml` (mirrored in `version.txt`). Tag to release:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

`.github/workflows/release.yml` builds the binaries `gpm2op self-update` pulls.
