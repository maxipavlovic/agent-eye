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
| `manual` | `eye backup`, and `eye forget` before it removes anything. |
| `pre-restore` | Before `eye backup restore` replaces the database. |
| `pre-import` | Before `eye import` changes the database. |

`[backup] enabled = false` in `config.toml` stops the daily and pre-upgrade ones; `eye backup` still
works. Kept: the newest `keep_daily` (7) daily snapshots, the newest daily of each of the last
`keep_weekly` (4) weeks, the last 3 `pre-v*`, 3 `pre-restore`, 3 `pre-import` and 5 `manual`. Only files named as
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

## Moving to another machine / keeping a copy in the cloud

```sh
eye export                       # agent-eye-<host>-<date>-<time>.eyepack in the current folder
eye export --to ~/Drive/eye/     # or a folder / file name of your choice
eye import agent-eye-laptop-20261121-1030.eyepack --dry-run   # the plan only
eye import agent-eye-laptop-20261121-1030.eyepack             # shows the plan, asks [y/N]
```

### Encrypting a pack

eye has no encryption of its own; `-` sends a pack through any tool you trust. With
[age](https://github.com/FiloSottile/age) and a passphrase:

```sh
eye export --to - | age -p > eye-backup.eyepack.age     # asks for a passphrase twice
age -d eye-backup.eyepack.age | eye import -            # on the other machine; asks [y/N] on the terminal
```

With GnuPG: `eye export --to - | gpg -c > eye-backup.eyepack.gpg` and
`gpg -d eye-backup.eyepack.gpg | eye import -`. `eye export --to -` refuses a terminal as its
output, prints its summary on standard error, and leaves no pack file behind; `eye import -`
saves the stream in eye's temp folder, checks it like a file and asks its question on the terminal
(add `--yes` in a script). `eye export <session> --to -` prints one session's page the same way.

A pack is one file (a zstd-compressed tar) with the database, the whole transcript archive and a
copy of the transcripts that are still with the agent but not archived yet, so it holds every
session eye knows, plus a manifest with the sha256 of each file. Upload the file to Google Drive
or any storage by hand; it is the same data as `~/.agent-eye`, so treat it that way (see
[security.md](security.md)): encrypt it before it leaves the machine (above), or keep it somewhere
only you can read. No `board.token` is in it.

`eye import` checks the checksums and the schema first (an older pack is migrated, a newer one is
refused), prints a plan and changes nothing until you say yes (`--yes` for scripts, `--dry-run` to
stop after the plan). It stops the daemon around the change, as `eye backup restore` does, and
keeps the old database as a `pre-import` snapshot (last 3 kept).

- **A machine with no sessions yet**: the pack's database becomes the database, and the archive
  and the live copies go into `~/.agent-eye/archive/`. A session whose transcript is not in the
  agent's folder here is restored from there when you resume it.
- **A machine with sessions**: sessions whose id is not here are added with their rows; a session
  that exists here keeps the local row unless the pack's last activity is newer; ideas are added
  when no idea has the same title, folder and creation time; archive files are copied when missing
  or older here. Hook events and read positions are not merged; the next scan rebuilds them.
- **Another home folder**: when the pack's home differs from this machine's (`/Users/max` on a Mac,
  `/home/max` on Linux) every path in the sessions and the ideas' folders is rewritten, including
  Claude's folder names (`-Users-max-code` becomes `-home-max-code`). The plan shows the rewrite.
  A session whose folder does not exist here still opens from the archive; the preview says so, and
  resume needs the folder.

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
