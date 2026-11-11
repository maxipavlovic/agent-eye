# Security

agent-eye is a local tool. This page says what runs, what listens, what is stored, what leaves the
machine, and how to report a problem.

## What runs

- `agent-eye` / `eye` when you start it: the picker, the command line, the board when you ask.
- The **daemon**, a background service that `eye install` adds for your user (not root): a launch
  agent on macOS, a systemd user unit on Linux. It reads the agents' files, updates the database,
  writes summaries and archive copies, and serves the board when it is turned on.
- **Hooks**: `eye install` adds one entry per event to the agent's hooks file (for Claude Code
  `~/.claude/settings.json`, for Codex CLI `~/.codex/hooks.json`). Each call is
  `<path>/agent-eye hook <Event>` (with `--agent codex` for Codex); it reads JSON on stdin,
  appends one short line to a local file, prints nothing and exits 0. Other tools' entries in that
  file are never touched, a backup is made, and `eye uninstall` puts the file back. A settings file
  that is not 2-space JSON is rewritten in that form (same entries, new layout), after the backup;
  install says so. Codex runs a hook only after you trust it once with `/hooks`; eye never touches
  Codex's trust or its `notify` setting. For Codex the `PostToolUse` hook runs after every tool call
  and Codex hands it the tool's input and output; eye keeps only the session id from it, so no
  command, file or output is stored.
- Resume and Start drive iTerm2 with AppleScript (macOS only; it asks permission once), open a
  `.command` file with `open` (macOS), run a terminal program (`xdg-terminal-exec`,
  `x-terminal-emulator` or `$TERMINAL`; Linux) or copy the command, with `pbcopy` on macOS and
  `wl-copy`, `xclip` or `xsel` on Linux. The `.command` file is made in `~/.agent-eye/tmp/` (mode
  0700, owner only) and holds the same quoted line that iTerm would be given.

eye never needs root and never runs `launchctl` or `systemctl` for anything but its own service.

## What listens where

Nothing, unless you turn the board on (`eye board on`, or `eye serve` once). Then one HTTP server
listens on `127.0.0.1:<port>` (default 7420) and on no other address. Every request is checked:

1. **Host**: only `127.0.0.1:<port>`, `localhost:<port>` and `[::1]:<port>` are served. A web page on
   another name that was pointed at 127.0.0.1 (DNS rebinding) keeps its own name in `Host` and gets
   a 403, even with the right token.
2. **Token**: the secret in `~/.agent-eye/board.token` (32 random bytes, hex, mode 0600), as the
   `eye_token` cookie (HttpOnly, SameSite=Strict) or the `X-Eye-Token` header. It is compared in
   constant time. `eye board` prints the link with the token; the first visit trades it for the
   cookie and redirects, so it does not stay in the address bar.
3. **Origin**: a request that changes something (POST) must carry an `Origin` that is one of the
   loopback names above, so another website in your browser cannot make it.
4. The web page is served with a strict Content-Security-Policy (own files only, no framing), and
   request bodies are limited to 64 KiB. The API sets no CORS headers.

Anyone who can read your files as your user can read the token. That is the trust boundary: eye
protects against other websites and other users, not against other programs running as you.

## What is stored

All under `~/.agent-eye`, readable by your user only (folder 0700, files 0600; an older, wider
folder is tightened on the next run):

- `eye.db`: titles, first prompts, the recent text of prompts and replies (for search), outputs
  (pull request links, commits, folders), summaries, ideas, hook events (short fields: the last 300 characters of each finished reply, not masked, and what a permission prompt asks, secrets masked; no prompts and no tool output), kept for 30 days.
- `archive/`: a **full compressed copy of each transcript**. It holds whatever was in the session,
  including anything sensitive you pasted. It is not masked. Delete `~/.agent-eye/archive` (or run
  `eye uninstall --purge`) to remove it.
- `backups/`: compressed snapshots of `eye.db` (see [backups.md](backups.md)), the same data as the database, kept as long as the rotation says. Delete the folder to remove them; `[backup] enabled = false` stops new ones.
- `queue/` (the same hook lines, rotated at 1 MB; one older file is kept), `logs/`, `config.toml`, `board.token`, `ui.json` (board theme, accent colour and layout), `daemon.lock`, `work/`, an empty folder the summary runs in, and `tmp/`, where a copy of an archived session is made while its conversation is read and removed right after.

**A mirror folder** (`[backup] mirror`) is a copy outside `~/.agent-eye`. eye itself makes no network
call, but if that folder is synced (Dropbox, iCloud Drive, a network share) your sync tool carries
the snapshots off the machine, and with `mirror_archive` the transcripts too, which can contain
secrets. Pick a folder that stays where you want it.

eye does not read the credential files that sit next to an agent's live-session files (`*.key`).
`eye uninstall` keeps the data unless you add `--purge`.

## What is sent to the agent

Only for **summaries**, and only through the agent you already use (for Claude Code: `claude -p`,
run from an empty folder, with no tools, no MCP servers and no hooks, no transcript kept; for Codex
CLI: `codex exec --ephemeral --ignore-user-config --ignore-rules -s read-only` from an empty folder,
nothing kept). For each
quiet session eye sends a short excerpt: the title, the folder and branch, the first prompt (up to
2,000 characters), the agent's own recap of the session, the last 30 messages (each cut to 1,500
characters), one line per tool call (no file contents, no tool output), the outputs list, the names
of sub-agents, and the previous summary. Before it is sent, the
excerpt is **masked** for API keys, cloud and GitHub tokens, Slack tokens, JWTs, bearer tokens,
`password=`/`token=`/`secret=` values and private keys. Masking is best effort, not a guarantee.

Sessions that are hidden, archived, or in an excluded folder are skipped. `[summary] enabled =
false` turns summaries off completely, and then nothing is sent anywhere. Summaries use your agent's
plan.

## Commands built from session data

Titles, folders, ids and prompts come from files an agent or a person wrote, so they are treated as
untrusted:

- The folder and every argument of a resume, fork or start command are single-quoted unless they are
  plain words. A folder that is not absolute follows `--`.
- A session id is only used in a command when it is letters, digits, `-`, `_` and `.`, and does not
  start with `-` or `.`. A session folder with a control character in its name gets no resume or
  fork command at all; the board, `eye resume` and the picker say why.
- A first message that starts with `-` is prefixed so it cannot be read as an option.
- The picker starts the agent directly, without a shell. The text typed into iTerm is escaped for
  AppleScript and stripped of control characters. The same goes for the line in a `.command` file; a
  Linux terminal gets the line as one argument of `sh -c`, never through another shell.
- The terminal views and the plain output of `eye show`, `find`, `ls` and `pick --filter` replace control characters in session text (titles, folders, prompts, replies) with spaces, so it cannot move the cursor, retitle the window or write to the clipboard.

## Reporting a problem

Please do not open a public issue for a security problem. Open a **private security advisory** on the
GitHub repository (Security tab, "Report a vulnerability") with what you found and how to repeat it.
