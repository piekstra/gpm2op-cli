# gpm2op

Catch your **1Password** vault up to **Google / Chrome Password Manager** or
**Apple Passwords / Safari**.

`gpm2op` reads a Chrome/Google or Apple Passwords/Safari passwords CSV export
and **idempotently upserts** each login into 1Password through the official
[`op`](https://developer.1password.com/docs/cli/) CLI. By default it creates only the logins you don't have yet and leaves
everything else alone — so you can keep using Chrome/Google for phone↔laptop
sync and run this whenever you want your 1Password vault to catch up. Re-running
is safe: it only adds what's newly appeared.

```console
$ gpm2op sync ~/Downloads/GooglePasswords.csv --dry-run
Read 214 credential(s) from GooglePasswords.csv (Chrome / Google Password Manager). Target vault: "Private" (dry run).
Planned: 12 change(s) across the export (nothing was written).

$ gpm2op sync ~/Downloads/GooglePasswords.csv
Read 214 credential(s) from GooglePasswords.csv (Chrome / Google Password Manager). Target vault: "Private".
Done: 12 created, 0 updated, 0 unchanged, 202 left as-is, 0 failed.
202 row(s) matched an existing login and were not compared. Their passwords are still in Google Password Manager. --update --dry-run lists the logins --update would compare; --update --force reconciles them (it overwrites a passkey on a login it edits, see the README's Passkeys section), or --keep-all keeps both passwords.
Deleted the export at GooglePasswords.csv. Google Password Manager still holds every password in it, including what the notes above say did not reach 1Password.
```

The `Read …` line and the notes are on stderr; the rest is stdout.

## What moves, and how

| What | From Chrome / Google | From Apple Passwords / Safari |
| --- | --- | --- |
| Passwords | CSV export, `gpm2op sync` | CSV export, `gpm2op sync` |
| One-time codes (TOTP) | Not in the export: they live in Google Authenticator | The CSV's `OTPAuth` column, imported by `gpm2op sync` |
| Passkeys | Not exportable to a file | Not in the CSV. On iPhone or iPad, the Passwords app hands them to 1Password directly: see [Passkeys](#passkeys) |

No file export carries a passkey, so no CLI can move one.

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

## Export your passwords

### From Google / Chrome

- **Chrome:** `chrome://password-manager/settings` → **Export passwords**, or
- **Web:** [passwords.google.com](https://passwords.google.com) → Settings →
  **Export passwords**.

The CSV's header is `name,url,username,password,note`. A file with only
`url,username,password` (the columns Google documents) is read the same way.

### From Apple Passwords / Safari

- **Mac, Passwords app:** **File → Export All Passwords to File**, then
  **Export Passwords** and **Save**
  ([Apple](https://support.apple.com/guide/passwords/mchl35b12625/mac)).
- **iPhone or iPad:** **Settings → Apps → Safari**, below *History and Website
  Data* tap **Export**, select **Passwords**, then **Save to Downloads**
  ([Apple](https://support.apple.com/guide/iphone/iphf28f2e93e/ios)).
- **Safari on a Mac:** **File → Export Browsing Data to File** saves a `.zip`;
  unzip it and pass the passwords CSV inside. Delete both the `.zip` and the
  CSV afterwards.

The CSV's header is `Title,URL,Username,Password,Notes,OTPAuth`. Apple does not
export Wi-Fi passwords, passwords in a group someone else shared with you, or
Sign in with Apple accounts.

`gpm2op` tells the two formats apart by the header and says which it found
(`Read … (Apple Passwords / Safari)`, and `summary.format` in `--json`). A file
whose header is neither, or has both `name` and `title`, is refused (exit 2)
before `op` is called, and the
error does not repeat the header, since a file without one has a password in
its first line.

| Column | Becomes |
| --- | --- |
| `name` / `Title` | the item's title (the site's host when blank) |
| `url` / `URL` | the website |
| `username` / `Username` | the username |
| `password` / `Password` | the password; a row with none (Chrome writes these for a sign-in through another site) is imported without a password field |
| `note` / `Notes` | the notes |
| `OTPAuth` | a one-time password field, when the value is an `otpauth://` address; any other value is not imported: a real run names its row on stderr, and `--json` lists it in `summary.otp.ignored_rows` |

**Every export is plain text.** `gpm2op sync` deletes the files it read once a
run finishes with no failed rows (see [Security](#security)). It cannot delete
copies elsewhere: see [After the import](#after-the-import).

## Usage

```bash
gpm2op check                                   # confirm op is signed in + vault
gpm2op sync passwords.csv --dry-run            # preview: what would change
gpm2op sync passwords.csv                      # create missing logins
gpm2op sync passwords.csv --update --force     # also fix changed passwords (see Passkeys)
gpm2op sync passwords.csv --vault "Personal"   # choose a vault (default: Private)
gpm2op sync passwords.csv --keep-csv           # leave the export on disk afterwards
gpm2op sync Passwords.csv "Google Passwords.csv" --keep-all --tag import-2026-11
                                               # every distinct password, from both, tagged
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
  untouched and the row is counted under `host_only`. Outside `--dry-run` it
  needs **`--force`** (exit 6 without it, before `op` is called): `op` edits a
  login by replacing it from a JSON template, which overwrites a passkey
  stored on it, and `op`'s item JSON does not reliably show one. Pass
  `--force` once the logins it would change hold no passkey you need, or
  before passkeys are moved into 1Password; `--dry-run --update` lists every matched login it would compare and might
  rewrite; it reads no passwords and checks no passkeys.
  `gpm2op` still refuses to edit a login whose JSON does show a passkey, and
  that row fails.
- **`--dry-run`:** print the plan without writing anything. The export is kept.
- **Several exports in one run:** `sync` takes one or more CSVs, of either
  format. Their rows are matched together, so in the default mode the last row
  for a site and username wins across all of them; a password, one-time
  password or note the winning row lacks is taken from the earlier row. A
  failed row keeps every export of the run for the rerun. Name each export
  once: the same file given twice (by any path that resolves to it) is refused
  with exit 2 before `op` is called.
- **`--keep-all`:** for exports that disagree when nobody knows which password
  is current. Every distinct site, username and password becomes its own
  login. A row that repeats an earlier one (same site, username and
  password), in the same file or another, is imported once: its one-time
  password fills in a kept row that has none, and its note, when different,
  is added to the kept row's. A repeat with a different one-time password
  becomes a login of its own. A login already
  in the vault with the same site, username and password is not created again:
  keep-all reads the password of each existing login on the same site and
  username to compare. It never changes an existing login, and it cannot be
  combined with `--update`. A real run says on stderr how many site and
  username pairs came with more than one password (`summary.conflicting_logins`
  in `--json`); sign in with each to find the current one, then delete the
  others. A password that was already in 1Password is left as it was, without
  the run's tag, so search by site and username to see every candidate.
- **`--tag TAG`:** tags every login the run creates (repeat the flag for more
  than one tag; a tag cannot contain a comma). Tag an import so it can be
  found and undone:

  ```bash
  op item list --vault Private --tags import-2026-11                      # what it created
  op item list --vault Private --tags import-2026-11 --format json | op item delete -   # undo it
  ```

  `op item delete -` reads the item list on stdin, as 1Password's CLI
  reference describes. Look at the list before deleting.
- **The export is deleted** after a sync in which no row failed, including one
  that created nothing because every row matched an existing login. The export
  is a copy: Google Password Manager or Apple Passwords still holds every
  password in it, so deleting it loses nothing. When a row fails, the export is kept and stderr
  says so; fix the cause and rerun, and the clean rerun deletes it.
- **`--keep-csv`** keeps the export. A script that syncs the same file more
  than once (a second vault, a later `--update`) passes it on every run but the
  last.
- **Rows not checked against 1Password are counted.** A row that matched an
  existing login and was left as-is (default mode), a row with no username
  matched to the only login on its site by host alone (which may be another
  account, so it is never updated, `--update` included), a row with no URL
  matched to a URL-less login by title (also never updated), an earlier duplicate
  of a later row's site and username, and a row with no URL and no username
  (not synced at all, named by row number) are each counted, and named on
  stderr in human mode. Notes name rows by number, never by site, username or
  password. A one-time password or note that did not reach 1Password (its row
  matched a login that already exists) is counted too (`summary.otp.not_added`,
  `summary.notes_not_added`). The deletion line says 1Password holds every
  credential only when none of these happened. Before removing anything from
  Google Password Manager or Apple Passwords, export again and run
  `--update --force` to reconcile the first group, and check the `host_only`
  logins in 1Password by hand.
- **Output streams:** results are on stdout; progress (`Read N credential(s)
  …`) and notes are on stderr, in every mode.
- **`--json`:** stdout carries exactly one JSON document. `sync` gives
  `sync-result/v1`: `summary` with `format` (`chrome`, `apple` or `mixed`),
  `mode` (`upsert` or `keep-all`), `tags`, the counts, `repeats` and
  `conflicting_logins` (keep-all), `otp` (`added`, `not_added`,
  `ignored_rows`), `notes_not_added`, `csv_deleted` (every export deleted), `sources` (each
  export's `path`, `format`, `rows`, `unreadable_rows` and `deleted`), and
  `unverified` (`not_compared`, `host_only`, `title_only`, `deduped`,
  `unreadable`), then
  `items`, each with its `file` (position among the exports, from 0) and `row`
  (the header is row 1). The `--verbose` table shows the same row, as
  `export:row` (export counted from 1) when the run has several exports.
  `gpm2op --json check` gives `check-result/v1` (`vault`, `logins`).

### How matching works

A CSV row is considered "already in 1Password" when an existing **Login** item
in the target vault shares the same **host** (normalized: lowercased, `www.`
stripped) and **username** (case-insensitive). Different usernames on the same
site are treated as separate logins. A row with no URL matches a login with no
URL by its title (case-insensitive) instead of the host. A title is a label,
not a site, so such a match is never updated, `--update` included: it is left
as-is and counted under `title_only`. A row with neither a URL nor a title
never matches anything. CSV rows are
de-duplicated by (host, username), keeping the last occurrence. The upshot: **running twice makes
no second-run changes.** `--keep-all` matches on host, username **and**
password instead, so it too makes no changes the second time.

The default mode and `--keep-all` never edit an existing login; only
`--update --force` does (see its bullet above).

## Passkeys

A passkey cannot be written to a file, so neither the CSV nor `gpm2op` can
carry one. Apple Passwords can hand passkeys and passwords straight to
1Password through Apple's Credential Exchange, on **iPhone or iPad with iOS /
iPadOS 26 or later**:

1. Update 1Password on the iPhone or iPad to the latest version from the App
   Store, and sign in.
2. Open the **Passwords** app, tap the **⋯** (ellipsis) button, then **Export
   Data to Another App**
   ([1Password](https://support.1password.com/import/)).
3. Follow the prompts and choose **1Password**. If it is not in the list,
   1Password is not on the latest version.
4. In 1Password, open a few imported logins and check that their passkeys are
   there and which vault they landed in. 1Password's documentation does not
   say whether the vault can be chosen.
5. Sign in to one important site with its passkey from 1Password before
   relying on it.

The transfer copies; Apple Passwords keeps its passkeys. Apple Account passkeys
are not exported.

- **On a Mac:** 1Password for Mac does not receive a Credential Exchange
  import yet (a 1Password staff answer of 2026-02-13 says the iPhone or iPad
  is the way, with other platforms planned). Passwords on the Mac syncs
  through iCloud Keychain, so its passkeys are on the iPhone too: do the
  transfer there.
- **From Google Password Manager:** there is no transfer. Add a passkey in
  1Password on each site that offers one (the site's security settings →
  create a passkey, with 1Password as the passkey provider).
- **Passkeys and `--update`:** a template edit overwrites a passkey on the
  login, and `op` does not reliably show one. Run any `--update --force`
  before the passkey transfer; afterwards, change a password in the 1Password
  app instead. The default mode and `--keep-all` never edit a login, so they
  are safe in either order.

## After the import

1. **Delete every copy of the export.** `gpm2op sync` deletes the files it
   read after a clean run, and nothing else. Delete, then empty the trash
   for, each copy that exists anywhere else: the `.zip` from Safari, a copy
   in Downloads, an attachment in an email or chat, and any copy uploaded to a
   shared drive such as Google Drive, iCloud Drive or Dropbox. On Drive,
   remove it from **Trash** as well, and check **Shared with me** of anyone
   it was shared with; a file someone else owns can be deleted only by them.
2. **Keep Apple Passwords or Google Password Manager** until 1Password has
   been in use for a while; they are what the deleted export was a copy of.
3. With `--keep-all`, sign in with each conflicting login, keep the one that
   works, and delete the others. `op item list --tags …` finds the logins the
   run created; one that was already in 1Password has no tag, so search by
   site and username too.

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
  errors. Notes and errors name rows by number; only `--verbose` and `--json`
  list each row's title, site and username.
- **The tool deletes the CSV** once a non-dry-run sync finishes with no failed
  rows, because it holds every password in plain text and Google Password
  Manager or Apple Passwords keeps the originals. It removes only the
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

Copyright (C) 2026 piekstra. Licensed under the GNU Affero General Public
License, version 3 only (AGPL-3.0-only); see [LICENSE](LICENSE). Releases up to
and including v0.3.0 were published under MIT.
