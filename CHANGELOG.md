# Changelog

## v0.3.0 — 2026-11-23

- **`gpm2op sync` deletes the CSV export by default** once a non-dry-run
  finishes with no failed rows, including a run that created nothing. The
  export is a copy of what Google Password Manager still holds. It is kept,
  with a note on stderr, after `--keep-csv`, `--dry-run`, any failed row (the
  clean rerun deletes it), or an export with no usable rows. Only the regular
  file that was read is removed: a symlink, directory, pipe or `/dev/stdin` is
  left alone, and so is a file that changed during the run.
- **`--keep-csv`** keeps the export. A script that syncs the same file more
  than once, such as a second vault or a later `--update`, passes it on every
  run but the last; without it, the first run deletes the file.
- Rows not checked against 1Password are counted: rows matched to an existing
  login and not compared, rows with no username matched to the only login on
  their site, earlier duplicates of a later row, and rows with no URL and no
  username, which are not synced at all. Human mode names each
  group on stderr. The deletion line says 1Password holds every credential only
  when there are none.
- **`--update` no longer overwrites a login matched on host alone.** A row with
  no username used to update the only login on its site, which may be another
  account's. That login is now left untouched (no reveal, no edit), the row
  counts under `unverified.host_only`, and stderr says it was not updated
  because the match was on host alone. `sync --help` says so under
  `--update`. This narrows `--update` for anyone who relied on it to fix such
  a row.
- **The `Read N credential(s) from …` line is on stderr in every mode**, human
  mode included. It was on stdout, so a script that parsed it there no longer
  finds it.
- **`--json` stdout is exactly one JSON document.** `sync` emits
  `sync-result/v1`, which adds `schema`, `summary.csv_deleted` and
  `summary.unverified` (`not_compared`, `host_only`, `deduped`, `unreadable`). An export with
  no usable rows emits an empty `sync-result/v1` instead of a text line.
  `check` emits `check-result/v1` (`vault`, `logins`) instead of its human
  line.
- A failed row's `result` names op's own message (`failed: [ERROR] …`), not
  only "`op` reported an error:".
- `--update`'s `0600` temp file survives no stop signal short of `SIGKILL`.
  SIGINT, SIGTERM, SIGHUP and SIGQUIT are held while it exists and passed on
  to `op`, which gets 5 s to exit before it is killed. An edit still running
  after `GPM2OP_EDIT_TIMEOUT` seconds (default 120) is stopped and its row
  fails. A value that is not a whole number of seconds above 0 gets one
  warning on stderr and the default.

Earlier versions: see the GitHub releases.
