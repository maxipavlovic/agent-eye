# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## 0.2.0 - 2026-11-12

### Added

- Reminders: bring a session or an idea back on a date (`eye remind <id> <when>`, Ctrl-R in the picker, REMIND ME on the board). A due one moves to Needs you. Desktop notices are off until you run `eye remind notify on`.
- Background work: a session with a background shell, agent or loop stays in Running with one line saying what. A session waiting on a permission prompt goes to Needs you.
- Days worked: the panel, the picker preview and `eye show` list the days and active time spent on a session.
- Backups: a daily snapshot and one before each upgrade, `eye backup [ls|restore]`, and an optional mirror folder (`docs/backups.md`).
- An idea whose folder is missing offers to create it.
- Board: eight accent colours (also on the site), RESUME in the panel with the R key, `+N MORE` opens the full list, and a link to Remote Control sessions.

### Changed

- `eye ls` rows end with activity marks (`bg`, `↻`) after `[column]`. `--json` only gains fields.
- Database schema 12 (new columns and tables only; 0.1.x still reads it).

### Fixed

- Codex: pull request and Jira links returned by MCP tools now show in outputs.
- Linux: opening a terminal no longer fails now and then with "text file busy".

## 0.1.1 - 2026-11-10

### Changed

- Resume and Start open a terminal by default (`[board] resume = "auto"`): iTerm2, else Terminal on macOS, else a terminal window on Linux; otherwise they copy the command.

### Fixed

- Linux: restarting the service no longer closes terminals opened from the board.

## 0.1.0 - 2026-11-10

First release, for macOS and Linux, with Claude Code (default) and Codex CLI.

### Added

- A local index of your sessions with full-text and fuzzy search (`eye find`).
- A terminal picker (`eye`): preview, conversation view, resume, fork and copy. Ctrl-G opens it from your shell.
- Compressed copies of transcripts, put back on resume after the agent deletes the original.
- Live status from hooks, and short summaries written by your agent's own CLI (can be turned off).
- An optional board on `127.0.0.1` (`eye board on`) with a JSON API, and Ideas to park a task and start it later.
- `eye install`, `uninstall` and `doctor`, and an `/eye` skill.
- A shell installer and a Homebrew tap, with checksums and attestations.

### Privacy

- Everything stays on your machine: no telemetry, no account. Only summary excerpts go through your own agent CLI, with obvious secrets masked.
