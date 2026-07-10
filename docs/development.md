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
src/report.rs      # table / JSON / summary rendering (the output layer)
src/selfupdate.rs  # self-update via GitHub releases
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

## Testing

Unit tests are inline and offline (no `op`): CSV parsing, host normalization,
template shape, matching/plan, dedupe. The live write path needs an `op`
session; verify manually with `op item create --dry-run -` against a scratch
vault.

## Releasing

Version lives in `Cargo.toml` (mirrored in `version.txt`). Tag to release:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

`.github/workflows/release.yml` builds the binaries `gpm2op self-update` pulls.
