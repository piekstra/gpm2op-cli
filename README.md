# gpm2op

Catch your **1Password** vault up to your **Google / Chrome Password Manager**.

`gpm2op` reads a Chrome/Google passwords CSV export and **idempotently upserts**
each login into 1Password through the official [`op`](https://developer.1password.com/docs/cli/)
CLI. By default it creates only the logins you don't have yet and leaves
everything else alone — so you can keep using Chrome/Google for phone↔laptop
sync and run this whenever you want your 1Password vault to catch up. Re-running
is safe: it only adds what's newly appeared.

```console
$ gpm2op sync ~/Downloads/GooglePasswords.csv --dry-run
Read 214 credential(s) from GooglePasswords.csv. Target vault: "Private" (dry run).
Planned: 12 change(s) across the export (nothing was written).

$ gpm2op sync ~/Downloads/GooglePasswords.csv
Done: 12 created, 0 updated, 0 unchanged, 202 left as-is, 0 failed.
```

## Why passwords only

This first version does **passwords** — and only passwords — on purpose, because
that's the part that can actually be automated:

- **Passwords** — Google/Chrome exports them to CSV; this tool syncs them. ✅
- **TOTP / 2FA seeds** — live in **Google Authenticator**, not Password Manager,
  and Google exposes no CSV/API for them (export is a manual on-phone QR). A
  future `gpm2op totp` importer could decode that QR; it's not here yet.
- **Passkeys / security keys** — non-exportable by design; there's no way to
  migrate them programmatically. You re-register these per site in 1Password.

## Prerequisites

- **1Password CLI (`op`) 2.x**, installed and signed in. The smoothest setup is
  the desktop app's CLI integration (1Password → Settings → Developer →
  *Integrate with 1Password CLI*), which unlocks `op` with Touch ID.
  Verify with `gpm2op check`.
- A **Rust toolchain** (1.82+) to build.

```bash
git clone https://github.com/piekstra/gpm2op
cd gpm2op
cargo install --path .        # installs the `gpm2op` binary
```

## Export your passwords from Google/Chrome

- **Chrome:** `chrome://password-manager/settings` → **Export passwords**, or
- **Web:** [passwords.google.com](https://passwords.google.com) → Settings →
  **Export passwords**.

You'll get a CSV with the header `name,url,username,password,note`. **It's plain
text** — delete it once you've synced (the tool reminds you).

## Usage

```bash
gpm2op check                                   # confirm op is signed in + vault
gpm2op sync passwords.csv --dry-run            # preview: what would change
gpm2op sync passwords.csv                      # create missing logins
gpm2op sync passwords.csv --update             # also fix changed passwords
gpm2op sync passwords.csv --vault "Personal"   # choose a vault (default: Private)
gpm2op sync passwords.csv --json               # machine-readable summary
```

- **Default (no `--update`):** create logins that don't exist yet; never touch
  existing ones. This is the fast, safe "catch-up."
- **`--update`:** additionally reconcile — for a login that already exists, if
  the CSV password differs, update it (only then; identical passwords are left
  untouched and reported as `unchanged`).
- **`--dry-run`:** print the plan without writing anything.

### How matching works

A CSV row is considered "already in 1Password" when an existing **Login** item
in the target vault shares the same **host** (normalized: lowercased, `www.`
stripped) and **username** (case-insensitive). Different usernames on the same
site are treated as separate logins. CSV rows are de-duplicated by
(host, username), keeping the last occurrence. The upshot: **running twice makes
no second-run changes.**

## Security

This tool touches every one of your passwords, so it's built to be careful — and
the source is short enough to audit end to end:

- **Secrets never go on the command line.** `op` itself warns that command
  arguments are visible to other processes. New items are created by piping a
  JSON template to `op item create -` (**stdin**); updates go through a
  `0600` temp file that's deleted immediately. No password is ever an argv.
- **1Password stays the source of truth.** Every write is an `op` call; this
  tool stores nothing itself and keeps no state.
- **It never prints your passwords** — not in tables, not in `--json`, not in
  errors.
- **Delete the CSV** when you're done; it's plaintext. The tool reminds you
  after any create.

## Development

```bash
cargo test                       # parsing, matching, dedupe, templates
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Internals and design notes: [`CLAUDE.md`](CLAUDE.md).

## License

MIT — see [LICENSE](LICENSE).
