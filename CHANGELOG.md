# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- Desktop notices for due reminders are opt-in. `[remind] notify` now defaults to `false` (an old config without the key means off). `eye remind notify [on|off]` shows or switches it, `eye remind <id> <when>` says when notices are off, Tab in the picker's Ctrl-R input flips it, `eye doctor` prints the state, and the board gets `GET`/`POST /api/remind/settings`. The daemon follows a change at once. macOS shows these notices under Script Editor, which is why one appeared without a prompt; `docs/config.md` says where to turn it off.
- Backups of the database: `eye backup` makes a snapshot, `eye backup ls` lists them, `eye backup restore latest` (or a name from the list) puts one back after asking. The daemon makes one a day and eye makes one before a schema upgrade; the last 7 daily, 4 weekly and a few of each other kind are kept. `[backup] enabled = false` stops the automatic ones. An optional `mirror` folder (it must exist already) gets a copy of each snapshot, and `mirror_archive` the transcript archive too. `eye doctor` shows the newest snapshot, the mirror and a check of the live database. See `docs/backups.md`.
- An idea whose folder does not exist is no longer a dead end. Start asks to create the folder instead of opening a broken terminal: the board's button reads `CREATE FOLDER + START`, the idea form says under the folder field when it does not exist yet, and `eye idea start` and the picker ask first (`--create-folder` skips the question). Eye creates folders only inside your home folder; any other missing folder, or a file in the way, is refused with the reason. The API adds `folder_state` to idea cards, `GET /api/folder`, and `create_folder` on `POST /api/ideas/:id/start`; without it a missing folder answers 409 `folder_missing`.
- The board has eight accent colours: Vermilion (as before), Cobalt, Teal, Moss, Amber, Magenta, Violet and Graphite, each with a light and a dark pair, picked from the button next to the theme switch. Only the signal colour changes; the choice is saved with the other board settings (`eye.accent`). The terminal picker keeps your terminal's colours.
- Reminders: ask eye to bring a session or an idea back on a date, for the reply that does not come or the idea to look at by a day. A due reminder puts the card in Needs you, above a column you placed it in (never above the archive), with the line `check on: <your note>`; it clears when you go back to the session, mark it, or snooze it. Set it from the board (right-click, REMIND ME: tomorrow 9:00, Monday 9:00, in a week, or a date), in the panel, in the picker (Ctrl-R) or with `eye remind <id> <when> [--note TEXT]`; `eye remind ls` lists them. `<when>` is `tomorrow`, `mon`..`sun`, `30m`, `4h`, `3d`, `1w`, `2026-11-20` or `2026-11-20 14:00`, in local time. Nothing leaves the machine: the daemon can show one desktop notice when a reminder is due (`osascript` on macOS, `notify-send` on Linux), but only when you turn it on: `eye remind notify on`, Tab in the picker's Ctrl-R input, or `[remind] notify = true`.
- The API: `remind_at`, `remind_note` and `remind_due` on every card, and `POST /api/cards/:id/remind`. The database moves to schema version 11 (new columns only; an older eye still reads it).
- A session that runs something in the background or loops on its own stays in Running with one line saying what (`background: npm run dev · 4 min`, `↻ every 10 min · next 14:20`), on the board, in the picker (`bg`, `↻` marks), in `eye ls` and in `eye show`. Claude Code: background shells, agents and monitors, `/loop` schedules and self-paced loops. Codex CLI: background shells and an active `/goal`. A session that waits on a permission prompt or an input request goes to Needs you and says which; a dialog you opened yourself does not count.
- `eye ls` text rows end with the activity markers (such as `bg` and `↻`) after the `[column]` part, as the picker shows them; a script that matches the end of the line should match the `[column]` instead. `--json` is unchanged apart from the new fields.
- A small `RC` mark on sessions with Claude Code's Remote Control on, and a link to open them in the browser in the panel.
- API: cards have `activity`, `background`, `loops` and `remote_url` (see `docs/api.md`). Database schema 10 adds four columns; older versions of eye ignore them.
- Days worked: the panel's WHERE section shows `WORKED ON  6 days over 12 · 7 h 40 m active · last today 14:20` under STARTED, with a strip of the days (the last 60 when the span is longer). The picker preview and `eye show` print `worked 6 days · 7 h 40 m · last today 14:20`. Active time is the gaps between messages of up to 15 minutes; hours are kept in UTC and turned into local days when shown. Sessions eye read before this are counted in the background, a few per daemon tick, newest first, from their timestamps only (the transcript, or eye's archive copy when it is gone); nothing is read again and `eye scan` counts them all at once. The detail card has `worked` (see `docs/api.md`). Database schema 12 adds two tables; older versions of eye ignore them.

### Fixed

- The accent colours are one button in the board's header that opens a row of the eight, so the header keeps its height on a small screen. The `CREATE FOLDER + START` button wraps inside a narrow idea card instead of running past it.
- Opening a terminal on Linux no longer fails now and then with "text file busy" when the terminal program was written a moment before.

## 0.1.1 - 2026-11-10

### Changed

- `[board] resume` is now `"auto"` by default, on macOS and Linux: Resume and Start open an iTerm2 tab when iTerm2 is installed, else a Terminal window on macOS, and a new terminal window (`xdg-terminal-exec`, `x-terminal-emulator` or `$TERMINAL`) on Linux when there is a display; otherwise they copy the command. `"terminal"` is a new explicit choice; `"iterm"` and `"copy"` work as before. The board and the API say what a click will do (`ITERM`, `TERMINAL` or `COPY`).

### Fixed

- Linux: restarting the systemd service no longer closes a terminal that Resume or Start opened from the board (`KillMode=process`).

## 0.1.0 - 2026-11-10

First release for macOS and Linux. Claude Code (the default) and Codex CLI are the supported agents; the code has an agent layer so others can be added.

### Added

- A local index of your sessions, built from the agent's own files and kept in `~/.agent-eye`: titles, prompts, replies, folders, branches, pull requests, Jira-style keys and token use.
- Search over all of it: `eye find`, full-text and fuzzy, with recent and active sessions first.
- A terminal picker (`eye`) with a preview and a conversation view. Enter resumes a session, Ctrl-F forks it, Ctrl-Y copies the command.
- A shell key (`eval "$(eye init zsh)"`): Ctrl-G opens the picker and puts the chosen resume command on your prompt.
- A compressed copy of every transcript. When the agent has deleted the original, resume puts the copy back first.
- Live status from the agent's hooks, and short summaries (where it stands, the next step, who acts) written by the agent's own CLI in headless mode. Summaries can be turned off.
- An optional pixel-art board on `127.0.0.1` (`eye board on`) with the columns Needs you, Running, Waiting on others, Paused, Done and Ideas.
- Ideas: park a task, edit it, start it later as a new session.
- `eye install`, `eye uninstall` and `eye doctor`, and a `/eye` skill for Claude Code. The background process is a launch agent on macOS and a systemd user service on Linux. Install prints one short line per area (`--dry-run` and `--verbose` show the file changes) and restarts a running daemon on the new binary without taking it down first.
- On Linux, Resume and Start copy the command (macOS can open an iTerm2 tab), and copying uses `wl-copy`, `xclip` or `xsel`.
- Codex CLI as a second agent: its sessions (including archived ones, renamed ones and sub-agents), search, resume and fork, live status, hooks, summaries through `codex exec`, and the `/eye` skill. Codex runs hooks only after you trust them once with `/hooks`; it has no background sessions, so an idea starts in the foreground and is linked to the session that begins in its folder.
- `eye install` follows the agents it finds on the machine (`[agents] enabled`), and `[summary] model` is the agent's own cheap model when empty (`haiku` for Claude Code, `gpt-6-luna` for Codex CLI).
- With more than one agent enabled, cards, the panel, and the picker's rows and preview name the agent.
- A JSON API for the board (`docs/api.md`).
- Releases for macOS (Apple silicon and Intel) and Linux (x86_64 and aarch64, glibc) through a shell installer and a Homebrew tap, with checksums and build attestations.

### Privacy

- Everything stays on your machine. There is no telemetry and no account.
- The only text that leaves is the short excerpt a summary sends through your own agent CLI, with obvious secrets masked.
