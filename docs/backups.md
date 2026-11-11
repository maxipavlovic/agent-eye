# Backups

eye keeps compressed snapshots of its database, `~/.agent-eye/backups/eye-<date>-<time>Z-<reason>-s<schema>.db.zst`
(about a third of the database's size). The time in the name is UTC (the `Z`), and `s<schema>` is the schema version of the database inside, so `eye backup ls` never has to open a snapshot. The database holds what a rescan cannot bring back: your
ideas, manual column moves, hidden flags and the summaries. The transcript archive
(`~/.agent-eye/archive/`) is a separate copy of transcripts; see the mirror below.

## When a snapshot is made

| Reason | When |
|---|---|
| `daily` | The daemon makes one when the newest daily snapshot is over 24 hours old. |
| `pre-v<N>` | Before a new version of eye changes the database to schema `N`. If it cannot be made, eye says why on standard error and goes on. |
| `manual` | `eye backup`. |
| `pre-restore` | Before `eye backup restore` replaces the database. |

`[backup] enabled = false` in `config.toml` stops the daily and pre-upgrade ones; `eye backup` still
works. Kept: the newest `keep_daily` (7) daily snapshots, the newest daily of each of the last
`keep_weekly` (4) weeks, the last 3 `pre-v*`, 3 `pre-restore` and 5 `manual`. Only files named as
eye names them are ever deleted. See [config.md](config.md).

Snapshots are made through SQLite, not by copying the file, so one is consistent while the daemon
writes.

## Commands

```sh
eye backup                 # snapshot now, prints where and how big
eye backup ls              # name, reason, age, size, schema version; newest first
eye backup restore latest  # or a name from the list; shows what will happen, asks [y/N]
eye backup restore latest --yes  # for scripts (without it a non-terminal refuses)
```

`eye backup restore` checks the snapshot first (`PRAGMA quick_check` must say `ok`, and its schema must not
be newer than your eye), keeps the current database as a `pre-restore` snapshot, stops the daemon
when it runs from the service, replaces `eye.db` (and removes its old `-wal` and `-shm` files),
and starts the daemon again. A daemon you started by hand has to be stopped first, and so do other eye windows (the picker, `eye pick`): one that stays open keeps writing to the old database. Transcripts only
grow, so the next scan reads forward from the restored position and brings back the sessions that
are newer than the snapshot. What you changed by hand after the snapshot (ideas, column moves)
is not in it. (`eye restore <session id>` is a different command: it puts an archived transcript back.)

## Mirror (optional)

```toml
[backup]
mirror = "/Volumes/Backup/eye"   # absolute or ~/..., and must exist already
mirror_archive = true            # also copy the transcript archive
```

Each snapshot is copied into the folder and rotated there. A folder that is missing (a disk that is
not plugged in) is skipped and never created; the local snapshot is made anyway. With
`mirror_archive`, files of `~/.agent-eye/archive/` whose size or time changed are copied into
`<mirror>/archive/`; nothing is ever deleted there. A synced mirror folder carries the data off the
machine: see [security.md](security.md).

## Doctor

`eye doctor` shows the number and size of the snapshots and the age of the newest, warns when
automatic snapshots are on and the newest is over 3 days old, says when they are off, shows the
mirror (ok, folder missing, or why the last copy failed), and runs `PRAGMA quick_check` on the live
database.
