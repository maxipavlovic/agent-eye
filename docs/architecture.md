# Architecture

agent-eye is one Rust program (`agent-eye`, with `eye` as an alias) that keeps a local index of
coding-agent sessions. It has a terminal picker, a command line and an optional web board. Nothing
leaves the machine except the excerpts sent to your agent for summaries (see `security.md`).

## Data flow

```
 agent files            hooks                     eye
 ───────────            ─────                     ───
 transcripts ──┐   agent ─ `eye hook <Event>` ─▶ queue/events.jsonl
 live sessions ├─▶ scanner ──┐                           │
 background jobs│   (cursors, │                           ▼
 prompt history ┘    parsers) ├────────────▶ SQLite (eye.db) ◀── daemon: events, waits,
                              │                    │             summaries, archive
                              │                    ├──▶ picker (`eye`, Ctrl-G)
        archive (zstd) ◀──────┘                    ├──▶ CLI (`eye find`, `ls`, `show` ...)
                                                   └──▶ board server ──▶ face (browser)
```

1. **Agents write their own files**: transcripts, live-session files, background-job files, a
   prompt history. eye only reads them (and the archive writes copies elsewhere).
2. **The scanner** (`src/scan/`) asks each enabled agent for its transcripts (a file, a folder or a
   database each, with a cheap change stamp), skips the ones whose stamp did not move, and hands the
   rest to the adapter with the read marker it returned last time (a cursor per transcript). The
   adapter reads only what is new, or everything again when it must, and says which; the scanner
   stores sessions, outputs, tokens, streams and search text in SQLite. The picker runs a quick scan
   before it opens; the daemon scans every two seconds.
3. **Hooks** (`src/hook.rs`) are tiny: the agent calls `eye hook <Event>` with JSON on stdin, the
   adapter keeps a few short fields, and one line is appended to `queue/events.jsonl`. The hook
   prints nothing and always succeeds.
4. **The daemon** (`src/daemon.rs`, kept running by a launch agent on macOS and a systemd user service on Linux) drains the queue (who waits on you, what they
   ask), keeps live status fresh, writes summaries for quiet sessions, keeps compressed copies of
   transcripts, and serves the board when it is turned on.
5. **Views**: the picker (`src/tui/`, `src/pick.rs`), the CLI (`src/cli/`) and the board server
   (`src/server/`, JSON API in `api.md`, the web face in `face/` embedded in the binary).

## Modules

| Module | Does |
|---|---|
| `agents/` | The `Agent` trait, the common model (`Delta`, `LiveSession`, `Job`, `Invocation` ...), the registry, and `append_only`, the helpers for a session that is one growing file. `agents/claude/` is the Claude Code adapter and `agents/codex/` the Codex CLI one. See `agents.md`. |
| `scan/` | Walks transcripts with cursors (stamp and marker); stores sessions, sub-agents and tokens, history-only sessions, jobs and live status, Jira keys. |
| `db.rs`, `model.rs` | SQLite schema with migrations; the session, idea, stream and output types. |
| `columns.rs`, `hide.rs` | Which column a session is in; which sessions are throw-away or belong to a switched-off agent. |
| `search.rs`, `pick.rs` | Full-text search and ranking; the picker's index, ranking and launching. |
| `summarize.rs`, `summarize_prompt.txt` | Builds the masked excerpt of a session, asks the summary agent, stores the answer. |
| `remind/` | "Check on this" reminders: the reading of a time (`when.rs`: `tomorrow`, `mon`, `4h`, `2026-11-20 14:00`, in local time), when one is due, setting, clearing, listing, and telling about due ones once. The column rule is in `columns.rs`; the notice itself is in `platform/notify.rs`. |
| `worked/` | The days a session was worked on: adapters report the time of every message (`Delta::events`), `store.rs` adds them into UTC hours (`worked_hour`) with the 15-minute away rule, `summary.rs` turns hours into local days for the card and the terminal, and `backfill.rs` counts sessions read before eye kept hours, from their timestamps only. |
| `ideas.rs`, `mark.rs` | Parked ideas and starting them; putting a session in a column by hand. |
| `folder.rs` | Whether an idea's folder exists, and the one rule for making it: only inside the home folder. |
| `archive.rs`, `conversation.rs` | Where and when sessions are copied to the archive and put back (how is the adapter's call); reading a conversation from the session or from a temporary copy of the archive. |
| `hook.rs`, `daemon.rs`, `install/`, `report.rs` | The hook entry point; the background process; install, uninstall and doctor (`install/` also holds the choice of agents and what is said about the daemon's service); the one-line-per-area summary that install, uninstall and `board on\|off` print. |
| `platform/` | Everything that differs between macOS and Linux, and nothing else does: the service manager (`service.rs` builds the launchd plist or the systemd unit, knows their commands and runs them, `launchd.rs`, `systemd.rs`), opening a URL, the clipboard tools, the desktop notice (`notify.rs`), and the terminal a resume opens (`terminal.rs`: what `[board] resume` comes to on this machine, and the `.command` file or terminal program that runs the command). See "Platforms" below. |
| `server/` | The board API: router and state, guard (token, Host, Origin), face files, cards, board, ideas, prefs. |
| `tui/`, `cli/` | The picker screen (state, input, render, terminal); the command line. |
| `config.rs`, `paths.rs`, `files.rs`, `util.rs`, `resume.rs` | Config; eye's home; safe file writes; helpers (quoting, ids, formatting); opening a command in an iTerm2 tab or a terminal window, or copying it. |

## Storage

Everything is under `~/.agent-eye` (mode 0700): `eye.db` (SQLite, WAL), `archive/`, `queue/`,
`logs/`, `config.toml`, `board.token`, `ui.json`, `daemon.lock`, `work/` (an empty folder the
summaries run in), `tmp/` (a copy of an archived session, only while its conversation is read; the `.command` files Resume opens in Terminal on macOS, removed after a day). The
database holds one row per session (with the agent that owns it), streams (sub-agents, background
sessions, topics, outputs), outputs, ideas, hook events, read cursors (change stamp and the
adapter's opaque marker), the hours each session was worked in (`worked_hour`, with `worked_state` saying which sessions are counted) and a full-text index.

## Platforms

macOS and Linux share all the code except `src/platform/`, which has the three things that differ:

| | macOS | Linux |
|---|---|---|
| Background service | launch agent `dev.agent-eye.daemon` in `~/Library/LaunchAgents`, run with `launchctl` | systemd user unit `agent-eye.service` in `~/.config/systemd/user`, run with `systemctl --user` |
| Open a URL | `open` | `xdg-open` |
| Desktop notice (due reminders, opt-in) | `osascript`, text as arguments | `notify-send` when installed, else none |
| Copy text | `pbcopy` | the first of `wl-copy`, `xclip`, `xsel` that works |
| Resume and Start (`[board] resume`) | an iTerm2 tab if installed, else a Terminal window (a `.command` file opened with `open`), or the command copied | a window of `xdg-terminal-exec`, `x-terminal-emulator` or `$TERMINAL` when there is a display, else the command copied |

Both service managers are compiled on every OS and tested on every OS; `Manager::current()` is the
one place that picks by the OS. A manager builds the service file and knows its commands; a
`Runner` runs them, and `install.rs` decides when that may happen. It never runs `launchctl` or
`systemctl` for a service folder other than the real one of the account (found with `getpwuid`, not
`$HOME`), in a dry run, or with `--no-service`, so tests and sandboxes cannot touch a real service.
Tests put stand-in programs on a search path.

Starting the daemon never leaves it down. A service that is running from a file that did not change
is restarted in place (`launchctl kickstart -k`, `systemctl restart`), so it runs the newly
installed binary. A changed launch agent is booted out, eye waits until `launchctl print` no longer
shows it (`bootout` answers before the service is gone, and a `bootstrap` that comes too early
fails with 5, Input/output error), and then bootstraps it, trying again a few times on that error.
A service that is not loaded is bootstrapped (systemd: `daemon-reload` and `enable --now`). The
result is checked with `launchctl print` or `systemctl is-active`. What these commands print is
captured and never shown; a failure is reported in one line and `install` exits with an error.

The hooks and the service run eye from where Homebrew or cargo keeps it, when it came from there
(`<prefix>/opt/agent-eye/bin`, which every `brew upgrade` moves along, or cargo's bin folder; see
`install/package.rs`), and eye never copies, relinks or deletes those files. Any other binary is
copied to `~/.local/bin`. The daemon looks at the file it was started from at every tick. When an
upgrade has replaced it (the same new file at two looks in a row), it stops as it does on SIGTERM
(board requests still open get two seconds) and execs the new binary with the same arguments, in the same process (`daemon/upgrade.rs`).

`install`, `uninstall` and `board on|off` print a short summary (`report.rs`): one line for each
area (`agents`, `binary`, `claude`, `codex`, `daemon`, `board`), home paths as `~`, and what the
person has to do next as a `Next:` line at the end. The diff of each changed file is kept as the
detail of its line and printed under it for `--dry-run` and `--verbose`.

## Design rules

- The core does not depend on a particular agent: a second agent is a new adapter, not a change
  to the scanner, board or picker. `tests/agents.rs` proves it with a fake agent. What still names
  Claude Code outside `agents/claude/` is compatibility with what shipped before there were
  several agents: the default of `[agents] enabled`, the `claude` id for stored rows and hooks that
  name no agent (`LEGACY_ID`), and the `--claude-dir` flag and `EYE_CLAUDE_DIR`.
- The agents' own files are theirs and can change with any release: they are read defensively, and
  `eye doctor` says when they stop parsing.
- The hook path stays tiny and silent; heavy work happens in the daemon.
- Reading is incremental and cheap: the picker opens in tens of milliseconds. A session is only
  opened when its stamp moved, and the adapter, which knows its storage (a growing file, a rewritten
  file, a folder, a database), decides how much to read again.
- Local only: the board listens on 127.0.0.1 and checks a token, the Host and the Origin.
