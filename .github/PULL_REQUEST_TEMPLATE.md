## What & why

<!-- What does this change and why? -->

## How verified

<!-- Tests added/updated? `make check` passing? Any live check against a scratch vault? -->

## Checklist

- [ ] `make check` passes (fmt, clippy `-D warnings`, tests, build)
- [ ] No password ever reaches argv, stdout, logs, or a test fixture
- [ ] Docs/README updated if the command surface changed

## Family / cli-common

- [ ] No shared/reusable behavior copied in that belongs in [cli-common](https://github.com/piekstra/cli-common) (`pk-cli-*`)
- [ ] Surface, DTO, or exit-code changes reflected in cli-common `DESIGN.md` / `conformance.md`
