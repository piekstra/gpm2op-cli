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
Read 214 credential(s) from GooglePasswords.csv. Target vault: "Private".
Done: 12 created, 0 updated, 0 unchanged, 202 left as-is, 0 failed.
202 row(s) matched an existing login and were not compared. Their passwords are still in Google Password Manager; run with --update to reconcile.
Deleted the export at GooglePasswords.csv. Google Password Manager still holds every password in it, including the rows above that were not checked against 1Password.
```

The `Read …` line and the notes are on stderr; the rest is stdout.

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
git clone https://github.com/piekstra/gpm2op-cli
cd gpm2op
cargo install --path .        # installs the `gpm2op` binary
```

## Export your passwords from Google/Chrome

- **Chrome:** `chrome://password-manager/settings` → **Export passwords**, or
- **Web:** [passwords.google.com](https://passwords.google.com) → Settings →
  **Export passwords**.

You'll get a CSV with the header `name,url,username,password,note`. **It's plain
text**, so `gpm2op sync` deletes it once a run finishes with no failed rows (see
[Security](#security)).

## Usage

```bash
gpm2op check                                   # confirm op is signed in + vault
gpm2op sync passwords.csv --dry-run            # preview: what would change
gpm2op sync passwords.csv                      # create missing logins
gpm2op sync passwords.csv --update             # also fix changed passwords
gpm2op sync passwords.csv --vault "Personal"   # choose a vault (default: Private)
gpm2op sync passwords.csv --keep-csv           # leave the export on disk afterwards
gpm2op sync passwords.csv --json               # machine-readable summary
gpm2op self-update                             # update the binary in place
```

- **Default (no `--update`):** create logins that don't exist yet; never touch
  existing ones. This is the fast, safe "catch-up."
- **`--update`:** additionally reconcile — for a login that already exists, if
  the CSV password differs, update it (only then; identical passwords are left
  untouched and reported as `unchanged`). It updates only a login matched on
  host **and** username: a row with no username that matches the only login on
  its site by host alone may be another account's, so that login is left
  untouched and the row is counted under `host_only`.
- **`--dry-run`:** print the plan without writing anything. The export is kept.
- **The export is deleted** after a sync in which no row failed, including one
  that created nothing because every row matched an existing login. The export
  is a copy: Google Password Manager still holds every password in it, so
  deleting it loses nothing. When a row fails, the export is kept and stderr
  says so; fix the cause and rerun, and the clean rerun deletes it.
- **`--keep-csv`** keeps the export. A script that syncs the same file more
  than once (a second vault, a later `--update`) passes it on every run but the
  last.
- **Rows not checked against 1Password are counted.** A row that matched an
  existing login and was left as-is (default mode), a row with no username
  matched to the only login on its site by host alone (which may be another
  account, so it is never updated, `--update` included), an earlier duplicate
  of a later row's site and username, and a row with no URL and no username
  (not synced at all) are each counted, and named on stderr in human mode. Before removing anything from Google Password Manager,
  export again and run `--update` to reconcile the first group, and check the
  `host_only` logins in 1Password by hand.
- **Output streams:** results are on stdout; progress (`Read N credential(s)
  …`) and notes are on stderr, in every mode.
- **`--json`:** stdout carries exactly one JSON document. `sync` gives
  `sync-result/v1`: `summary` with the counts, `csv_deleted`, and `unverified`
  (`not_compared`, `host_only`, `deduped`, `unreadable`), then `items`. `gpm2op --json check`
  gives `check-result/v1` (`vault`, `logins`).

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
  `0600` temp file that's deleted as soon as `op` returns — when `op` fails
  too, and when the run is interrupted (Ctrl-C, `SIGTERM`, `SIGHUP`) mid-edit:
  the signal is held until the file is gone and passed on to `op`. An edit
  still running after `GPM2OP_EDIT_TIMEOUT` seconds (default 120; an
  unanswered 1Password prompt) is stopped and its row fails. A value that is
  not a whole number of seconds above 0 gets a warning on stderr and the
  default. Only `SIGKILL`
  can leave the file. No password is ever an argv.
- **1Password stays the source of truth.** Every write is an `op` call; this
  tool stores nothing itself and keeps no state.
- **It never prints your passwords** — not in tables, not in `--json`, not in
  errors.
- **The tool deletes the CSV** once a non-dry-run sync finishes with no failed
  rows, because it holds every password in plain text and Google Password
  Manager keeps the originals. It removes only the
  regular file it read: a symlink, a directory, a pipe or `/dev/stdin` is left
  alone with a note on stderr, and so is a file that changed while the sync ran
  (it may hold rows the run did not read) or one with no usable rows (it may not
  be a password export). Anything left on disk is yours to delete.

## Updating

```bash
gpm2op self-update --check   # is a newer release available?
gpm2op self-update           # download it and replace the binary in place
```

What changed in each version: [`CHANGELOG.md`](CHANGELOG.md).

Releases are produced by pushing a version tag, which triggers the release
workflow that builds the binaries `self-update` pulls; see
[Releasing](docs/development.md#releasing). Package-manager installs should
upgrade via that manager.

## Development

```bash
cargo test                       # unit tests, plus end-to-end runs against a stand-in op
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Internals and design notes: [`CLAUDE.md`](CLAUDE.md).

## License

MIT — see [LICENSE](LICENSE).
