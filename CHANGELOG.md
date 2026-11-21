# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- `eye forget <id>... | --folder <path> [--transcript] [--yes] [--dry-run]` removes sessions from eye (the session, its search text, its other rows and its archive copy) and, with `--transcript`, the agent's own copy: Claude Code's transcript and sub-agent folder, Codex's rollout and its `session_index.jsonl` line, agy's conversation folder, database and summary row, OpenCode's rows or JSON files. It asks first, refuses a session that is open now and takes a `manual` backup snapshot before removing anything. The board has FORGET… in the card menu and the panel (key X) with a confirm and a box for the agent's copy (`POST /api/sessions/:id/forget`).
- Where a session was started: T3 Code, Conductor, the desktop app, an editor, the web or a headless run. A chip (App, Headless, Desktop, IDE, Web, Night) on the card and the panel for anything but the terminal, "started from: an app (sdk-ts)" in the picker preview, `origin` and `origin_raw` in `eye ls --json` and the card JSON, and `eye ls --origin app`. Sessions indexed before are filled from the first lines of their transcripts, with no re-read. `docs/agents.md` has the table.
- Loose ends: each session lists what it left behind. Servers and shells still running (on a port, or in the session's folder or scratchpad), containers, git worktrees in a scratchpad or whose folder is gone, commits on no remote and modified files nobody committed. A card shows "2 loose ends"; the panel has a LEFT BEHIND section; the picker's preview has a line and Ctrl-L opens the list; `eye loose` lists them and `eye loose stop|remove|ignore <key>` and `eye loose send <session>` act. Stop sends SIGTERM to a process whose pid and start time still match; remove runs `git worktree remove` without `--force`; commits and files only get a command to copy. Every action asks first. `[loose] enabled` and `days` in `config.toml`.
- Antigravity CLI (`agy`) and OpenCode as agents (`[agents] enabled = ["agy", "opencode"]`; `eye install` turns on the ones it finds). agy: read from its JSON log, with titles, tokens by model, history, archive and summaries; no hooks or live status. OpenCode: checked on OpenCode 2.3.2, with its older database and file layouts read too; no hooks, summaries or live status. New `Agent::transcript_exists` for sessions that live inside a database.
- Night shift (off until `[night] enabled = true`): plan an idea ("TONIGHT") or a session ("CONTINUE TONIGHT") and the daemon starts it, headless, inside a window (01:00-07:00 by default) when no session you work in is live. Each run works in its own git worktree on a new branch (or a copy of a plain folder), and runs in parallel with others up to `[night] parallel`. A Claude run's Bash can reach only package registries (`[night] network`; no git host) and `git push`, `gh`, `ssh` and the publish commands are denied; Codex runs have no network. Claude's own tools (WebFetch, MCP) are not limited by that list. Each run can set one limit (weekly plan above N%, a token budget, or minutes); a limit eye cannot check is refused when planned. A run whose plan runs out can start again on the other agent (`--switch`). In the morning the run's session is a Needs-you card with a NIGHT chip, its report, diff stat and loose ends, and RESUME, OPEN FOLDER and DISCARD. `eye night on|off|add|ls|cancel|log|window|parallel|run-now|discard`, a Tonight chip and list on the board, `/api/night`, and `docs/night.md`. A `codex exec` rollout is a session only when it ran in a night run's folder.
- Night shift has two ways to keep your folder safe: `worktree` (the default, as before) and `copy`, a separate local clone with no `origin` where your folder is locked completely, `.git` included. `[night] isolation`, `eye night add --isolation worktree|copy`, and a **Work in** choice in the TONIGHT and CONTINUE TONIGHT forms; the NIGHT RUN section shows which one a run used. For a repo copy, **BRING INTO MY REPO** (`eye night bring <run>`, `POST /api/night/:id/bring`) fetches the night branch into your repo after you confirm, and refuses when the branch exists. `night_run.isolation` is a column of schema 17. `docs/night.md` explains when to pick which.
- `eye usage [--days N | --week | --month] [--by project|agent|model|session] [--json]`: tokens and an API-equivalent cost estimate (not your bill) per day, project, agent, model or session, with plan limits on top. Prices live in one file with their sources; `[usage.prices]` adds or overrides a model, and a model without a price shows tokens and no cost. Sessions from before are counted in the background.
- A USAGE panel on the board (this week by project and by agent, the top 10 sessions, four weeks, the limit lines) and `GET /api/usage`; tokens and cost on the session panel, in `eye show` and in the picker preview.
- Codex plan limits, kept as the newest seen. For Claude Code, an opt-in `eye statusline [--wrap '<cmd>']` status line command stores the limits Claude Code hands to its status line; `eye install` does not turn it on (see `docs/usage.md`).
- `eye export` writes one `.eyepack` file with everything eye knows (database, transcript archive, and copies of transcripts the agent still has), and `eye import <file>` reads it on this machine or another one: a fresh install takes the pack's database, an existing one gets the missing sessions and ideas merged in (the newer row wins). Home folders are rewritten for the new machine (`/Users/max` to `/home/max`, also in Claude's `-Users-max-…` folder names). Import shows the plan first, asks `[y/N]` (`--yes`, `--dry-run`), and keeps a `pre-import` snapshot. The board's token is never in a pack. `eye export --to -` and `eye import -` stream a pack through standard output and input, so `age -p` or `gpg -c` can encrypt it on the way (eye has no encryption of its own).
- `eye export <session> [--html|--md]` writes one session as a readable, self-contained page (secrets masked, tool calls folded to a line), from the archive when the agent deleted the transcript. The board's session panel has an EXPORT button (`GET /api/sessions/:id/export`).
- The terminal preview says when a session's folder does not exist on this machine.
- Database schema 14 (`loose` and `loose_ignore`).
- Database schema 15 (`usage_day`, `usage_state`).
- Database schema 16 (`night_run`).
- Database schema 17 (`session.origin`, `session.origin_raw`).
- Board panel keys: the existing N, W, P, D and R stay as they were; new are M (remind me), T (continue tonight), F (fork), C (copy the resume command), H (hide), L (loose ends), E (export) and X (forget). `?` lists them, and each button shows its letter.

### Changed

- Search is one engine for the picker, the board and `eye find`. Word forms match (`tokens` finds `token`), a mistyped word is fixed from your own sessions, and half of the words are enough. A pasted link finds the session that made it. Titles and rare words count most; status and age no longer lift a session, so old sessions are not pushed to the bottom. Archived sessions still come last.
- `score` in `eye find --json` and `/api/search` is now higher-is-better.
- Database schema 13 (the search index stems words; 0.2.x still reads it).
- Dependencies: axum 0.8, zstd 0.14, toml 1.2, jiff in place of chrono, vite 8.3.7 (face).
- `config.toml`: a section or key this version does not know is listed once on standard error and ignored; text that is not TOML is still refused.
- Night runs: a run is claimed before it starts, so two starters or a cancel cannot start it twice; a run whose process start time cannot be read is marked lost and never signalled. In `acceptEdits` mode only build and test commands are allowed.
- `eye import` takes the `pre-import` snapshot before copying files, skips files that already match, and refuses a pack that unpacks to more than its manifest says. `POST /api/loose/stop|remove` need `confirm: true`. The usage backfill runs in its own thread.
- Going back to 0.2.x: remove the `[night]`, `[loose]` and `[usage]` sections from `config.toml` first (0.2.x refuses sections it does not know). 0.2.x still reads and writes the schema 17 database.

## 0.2.1 - 2026-11-12

### Fixed

- Homebrew and cargo: `agent-eye install` runs eye from where the package manager keeps it, not from a copy in `~/.local/bin`, so an upgrade reaches the hooks and the background service. A copy an earlier install left there becomes a link. Installed with Homebrew or cargo? Run `agent-eye install` once after upgrading.
- `agent-eye install --bin-dir` keeps an `eye` that already runs the same binary, such as Homebrew's own link, so the next `brew upgrade` can still link it.
- The daemon restarts itself when its binary is upgraded, instead of running the old version until it is restarted. It also stops within seconds while a board is open; an open board used to hold it up.

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
