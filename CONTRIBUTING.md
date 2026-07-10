# Contributing

Thanks for your interest. This is a personal project but PRs and issues are
welcome.

## Ground rules

- **Never print, log, or commit a password.** This tool touches every one of a
  user's credentials — secrets go to `op` via stdin/`0600` templates only, never
  argv, never stdout, never a test fixture.
- Keep the working tree clean: `make check` (fmt + clippy `-D warnings` + tests +
  build) must pass before you push.

## Workflow

1. Branch off `main`.
2. Add a test where it makes sense (parsing/matching/dedupe have inline
   `#[cfg(test)]` tests — all offline, no `op`).
3. Run `make check`.
4. Open a PR describing the change and how you verified it.

## Style

- Match the surrounding code; `cargo fmt` decides formatting.
- Comments explain *why* / non-obvious constraints, not *what*.
- See [`docs/development.md`](docs/development.md) for layout and invariants.
