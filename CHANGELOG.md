# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- Backups of the database: `eye backup` makes a snapshot, `eye backup ls` lists them, `eye restore latest` (or a name from the list) puts one back after asking. The daemon makes one a day and eye makes one before a schema upgrade; the last 7 daily, 4 weekly and a few of each other kind are kept. `[backup] enabled = false` stops the automatic ones. An optional `mirror` folder (it must exist already) gets a copy of each snapshot, and `mirror_archive` the transcript archive too. `eye doctor` shows the newest snapshot, the mirror and a check of the live database. See `docs/backups.md`.
- An idea whose folder does not exist is no longer a dead end. Start asks to create the folder instead of opening a broken terminal: the board's button reads `CREATE FOLDER & START`, the idea form says under the folder field when it does not exist yet, and `eye idea start` and the picker ask first (`--create-folder` skips the question). Eye creates folders only inside your home folder; any other missing folder, or a file in the way, is refused with the reason. The API adds `folder_state` to idea cards, `GET /api/folder`, and `create_folder` on `POST /api/ideas/:id/start`; without it a missing folder answers 409 `folder_missing`.
- The board has eight accent colours: Vermilion (as before), Cobalt, Teal, Moss, Amber, Magenta, Violet and Graphite, each with a light and a dark pair, picked with the swatches next to the theme switch. Only the signal colour changes; the choice is saved with the other board settings (`eye.accent`). The terminal picker keeps your terminal's colours.

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
