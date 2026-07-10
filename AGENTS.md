# AGENTS.md

Agent entrypoint for the **gpm2op** repository. This file is a thin index — the
substance lives in the documents it points to. Keep it and `CLAUDE.md` short.

## Start here

- **Development workflow & repo-local facts:** [`docs/development.md`](docs/development.md)
- **Architecture & internals:** [`CLAUDE.md`](CLAUDE.md) (peer of this file)
- **User-facing usage:** [`README.md`](README.md)

## Build, test, lint

```bash
make check   # fmt-check + clippy (-D warnings) + test + build — run before pushing
```

## Conventions

- Rust, `clap` derive; every write goes through the `op` CLI.
- **Secrets never on argv:** item creates pipe a JSON template over stdin;
  updates use a `0600` temp file. Never print a password.
- Put repo-specific guidance in `docs/development.md`, not here.
