# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## 0.1.0 - Unreleased

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
