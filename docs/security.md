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
  command, file or output is stored. For agy, `eye install` writes a global plugin `agent-eye`
  (`~/.gemini/config/plugins/agent-eye/`) with `SessionStart` and `Stop` hooks; each runs
  `agent-eye hook <Event> --agent agy`, prints `{}` and stores the same short fields (no prompt, no
  tool output). For OpenCode, see "The OpenCode plugin" below.
- Resume and Start drive iTerm2 with AppleScript (macOS only; it asks permission once), open a
  `.command` file with `open` (macOS), run a terminal program (`xdg-terminal-exec`,
  `x-terminal-emulator` or `$TERMINAL`; Linux) or copy the command, with `pbcopy` on macOS and
  `wl-copy`, `xclip` or `xsel` on Linux. The `.command` file is made in `~/.agent-eye/tmp/` (mode
  0700, owner only) and holds the same quoted line that iTerm would be given.

eye never needs root and never runs `launchctl` or `systemctl` for anything but its own service.

### Night shift

Off until `[night] enabled = true` **and** a run is planned. A planned run is something you wrote: the prompt is yours. What comes back (the report, the diff stat, the log) is untrusted output and is shown as text, never as markup.

A run starts an agent that edits files, unattended, so the guards are fixed in code and not settings:

- It never runs in your folder. Two modes (`[night] isolation`, or per run): `worktree` (the default), a git worktree on a new branch `night/<date>-<slug>` created from HEAD, or `copy`, a separate local clone on that branch with no `origin`, where your folder is denied completely (`.git` too) and the run writes only in the copy. A plain folder is a copy (at most 100 MB, without `node_modules`, `.git`, `target`, `.venv`, `dist` and symlinks). The known gap of `worktree`: your `.git` has to stay writable, so a command that builds a path in an unusual way can add a *new* top-level file in your folder (nothing existing changes); pick `copy` to close it. A copy's branch reaches your repo only through BRING INTO MY REPO (`git fetch`, after your confirm, refused if the branch exists).
- A night run cannot push. A sandboxed Claude run can reach only package registries from Bash (`sandbox.network.allowedDomains`, `[night] network`; no git host), so `git push` fails however it is written; `git push`, `git remote`, `gh`, `ssh`, `scp` and the publish commands (`npm publish`, `cargo publish`, `twine upload`, `gem push`, `docker push`) are also denied by rule. Claude's own tools (WebFetch, MCP) are not covered by that allowlist, and without the sandbox (`allow_unsandboxed`) Bash has no network limit. Codex runs with `--sandbox workspace-write`, which has no network. OpenCode has no sandbox: its run gets permission rules for the same commands and denies edits outside its folder, but a shell command can still write anywhere, so eye starts an OpenCode run only with `[night] allow_unsandboxed = true`. eye never passes `--force` or a bypass-permissions mode to anything.
- Claude's permission mode is `auto` (a classifier decides) or `acceptEdits` with an allow list of git status/diff/log/add/commit and build and test commands.
- A continued session remembers the paths of its original folder, and a sentence in the prompt does not stop it from writing there (it did, in a real test). So every Claude run is also confined, with `--settings`: Claude Code's sandbox for Bash (`sandbox.enabled`, `failIfUnavailable`, `allowUnsandboxedCommands = false`) writes only in the run's folder, the temp folder and the git folders a worktree commit needs; `sandbox.filesystem.denyWrite` and `Edit(//path/**)` deny rules cover the planned folder and the session's original working folder (deny beats any allow in your own settings); two `PreToolUse` hooks refuse the file tools and Bash when they name that folder. Claude's docs: sandbox writes default to "the current working directory and its subdirectories, any directories you've added with `--add-dir` ... plus the per-user temp directory"; "when the platform doesn't support the sandbox or the dependency check at startup fails, Claude Code runs commands without sandboxing" unless `failIfUnavailable` is set; a deny rule is evaluated before ask and allow, in every scope. Without a sandbox (Linux without `bubblewrap` and `socat`) a Claude run does not start, unless `[night] allow_unsandboxed = true`. Limits: reads are not blocked; network for Bash is limited to the registries of `[night] network`; in a repo the original `.git` has to stay writable for the worktree's commits, so the folder is denied entry by entry and a command that builds the path to a *new* file directly in the original folder gets past the hook (it cannot change anything that exists); a rule in a repo's own committed settings is not checked. See `night.md`.
- Codex runs use `--sandbox workspace-write`. Its docs: Codex can "read files, make edits, and run commands in the working directory automatically", the workspace includes "temporary directories like `/tmp`", `.git` stays read-only inside it, and "the default `workspace-write` sandbox mode keeps network access turned off".
- Runs are detached from the daemon (own session) and found again by pid and start time. A limit stops a run with SIGTERM to its process group, after checking pid and start time, and never with SIGKILL.
- Nothing leaves a copy except through BRING INTO MY REPO: one `git fetch <copy> <branch>:<branch>` in your repo, after your confirm, creating a new branch and refusing to replace one. It does not touch your checked-out branch or your files.
- The only thing eye deletes is on DISCARD, after your confirm, and only inside `<eye home>/night`: it removes the run's folder, runs `git worktree prune` and `git branch -D` on its own `night/` branch. It refuses while there are uncommitted changes unless the confirm says they will be lost. Nothing is ever deleted by the daemon by itself.
- agy and OpenCode runs start only with `[night] allow_unsandboxed = true`: neither confines a run's writes the way Claude's sandbox or Codex's `workspace-write` does, so the worktree or copy is their only guard.
- Night runs are headless sessions in eye's night folder, so they do not count as a session you work in. They keep your usual agent settings and hooks, so eye sees them like any other session.

The prompt, report and diff stat are kept in `night_run` in `eye.db`, as private as the rest of it. The report is masked like any summary.

### The OpenCode plugin

`eye install` writes one JavaScript file into OpenCode's config folder (`~/.config/opencode/plugins/agent-eye.js`) and one command
(`commands/eye.md`). OpenCode runs the plugin inside its own process, with your rights, so what it does matters: it subscribes to
OpenCode's events and for six of them (a session started, a turn started, a permission asked or answered, a turn ended) starts
`agent-eye hook <Event> --agent opencode` with a small JSON payload on stdin: the session id, the folder, the tool name and what
the permission is asked for (the command or the files, secrets masked and cut by eye before it is stored), how it was answered,
and the tail of the last reply. It reads no file, opens no connection, never changes or delays what OpenCode does (hooks run one
at a time in the background, each stopped after 5 s) and says nothing when something fails. The path of `agent-eye` in it is the
one installed. `eye uninstall` removes it, `eye doctor` checks it, and a file of that name that is not eye's is left alone.

### Leftovers

`[loose]` (on by default, `enabled = false` turns it off) makes the daemon look, every 5 minutes and after a session stops, for what sessions left behind. Looking never changes a repo or a process (it keeps what it finds in eye's own database): the process table of your own user (`ps` and `lsof` on macOS, `/proc` on Linux: pid, start time, command line, working folder and listening ports), `docker ps` when docker is installed, and in the repos of recent sessions `git worktree list`, `git remote`, `git log --branches --not --remotes` and `git status --porcelain`, each with `--no-optional-locks`, `core.fsmonitor=false` and a time limit. It writes nothing in any repo and starts nothing. The results (command lines and folder names included) are kept in `loose` in `eye.db`, which is as private as the rest of it, and shown on the board and in the picker like any other card text.

It acts only when you confirm one item: the page, the picker (Ctrl-L) or `eye loose stop|remove` asks, with the exact words of what will happen, and the board's API refuses a key no scan stored, and `stop` and `remove` without `confirm: true`. Stop sends SIGTERM (never SIGKILL) to a process after it re-reads it and finds the same pid and start time and your user as the owner, or runs `docker stop <id>` on a container whose id and creation time still match. Remove runs `git worktree remove` without `--force`, so git refuses a worktree with changes; for a record whose folder is gone eye only drops its own item and tells you to run `git worktree prune` yourself (it would touch the whole repo). eye never pushes, commits or deletes files for you: for commits and modified files it only gives you the command to copy. Sending a session back opens it with a first message built from the list, through the same Resume path as the Resume button.

### Forget

`eye forget` (and `POST /api/sessions/:id/forget`, which needs `confirm: true` and the board's token) acts only on sessions you name, after the list and a `[y/N]`. It removes rows of eye's own database and files inside eye's archive. With `--transcript` it also removes files inside the agent's own folder (the adapter refuses a path outside it): that is the only place eye deletes something the agent wrote, and it cannot be undone. A session that is open now is refused, and a `manual` snapshot of the database is taken first (a snapshot holds the session's rows, not the agent's files).

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
- The steering clock (`clock.md`) adds, to `eye.db`, when you and the agent took turns (`turn`: a time, a kind and a size such as a prompt's length or a reply's word count; never text), which prompts a model judged to be corrections (`correction`: the verdict, a few words naming the complaint, the model, a hash of what was judged) and, with `[clock] idle = true`, the stretches of 5 minutes or more when the machine had no keyboard or mouse input (`idle_span`: a start and an end).
- `archive/`: a **full compressed copy of each transcript**. It holds whatever was in the session,
  including anything sensitive you pasted. It is not masked. Delete `~/.agent-eye/archive` (or run
  `eye uninstall --purge`) to remove it.
- `night/`: the night runs' worktrees or copies, logs and exit codes, until you DISCARD a run.
- `backups/`: compressed snapshots of `eye.db` (see [backups.md](backups.md)), the same data as the database, kept as long as the rotation says. Delete the folder to remove them; `[backup] enabled = false` stops new ones.
- `queue/` (the same hook lines, rotated at 1 MB; one older file is kept), `logs/`, `config.toml`, `board.token`, `ui.json` (board theme, accent colour and layout), `daemon.lock`, `work/`, an empty folder the summary runs in, and `tmp/`, where a copy of an archived session is made while its conversation is read and removed right after.

**A pack** (`eye export`, a `.eyepack` file) holds the database, the whole archive and a copy of
every transcript that was not archived yet: everything above except `board.token`, the queue and
the logs. It is written 0600 where you ask (the current folder by default) and leaves the machine
only when you move it. eye does **not encrypt** it and the transcripts in it are not masked: send
it through `age -p` or `gpg -c` with `eye export --to -` before it goes to cloud storage (see
[backups.md](backups.md)), or keep it where only you can read it. `eye import` checks every file's sha256 before it changes anything
and refuses a pack from a newer eye. The readable page of one session (`eye export <session>`,
EXPORT on the board) masks secrets the way summaries do, which is best effort.

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
nothing kept; for agy: `agy -p --output-format json --mode plan --sandbox` with a JSON schema, and the conversation it makes is deleted afterwards (folder, database and its row in `conversation_summaries.db`; `history.jsonl` only while agy is closed); for
OpenCode: `opencode run --format json --archived` with permission rules that deny the shell, edits, web fetches and folders outside its own empty one, and the session it makes is deleted afterwards). For each
quiet session eye sends a short excerpt: the title, the folder and branch, the first prompt (up to
2,000 characters), the agent's own recap of the session, the last 30 messages (each cut to 1,500
characters), one line per tool call (no file contents, no tool output), the outputs list, the names
of sub-agents, and the previous summary. Before it is sent, the
excerpt is **masked** for API keys, cloud and GitHub tokens, Slack tokens, JWTs, bearer tokens,
`password=`/`token=`/`secret=` values and private keys. Masking is best effort, not a guarantee.

Sessions that are hidden, archived, or in an excluded folder are skipped. `[summary] enabled =
false` turns summaries off completely, and then nothing is sent anywhere. Summaries use your agent's
plan.

### The steering clock

`[clock] idle = true` makes the daemon read, once a minute, how long it has been since the last keyboard or mouse input: on macOS the `HIDIdleTime` value from `ioreg -c IOHIDSystem`, on Linux the `IdleHint` and `IdleSinceHint` of your graphical session from `loginctl show-session` (the session is found with `loginctl show-user`). It reads that one number. It never sees which keys, which window or which application, and nothing leaves the machine. Only stretches of 5 minutes or more are stored (start and end). It is off by default; `eye doctor` says whether the daemon can read it and whether it is on.

The corrections pass (`[clock] llm`, on when summaries are on) sends **prompts of 1,500 characters or fewer that contain a word like "no", "wrong" or "again", each with the last 700 characters of the reply before it**, masked the way summary excerpts are, to the agent and model that write summaries, through the same call, so what it can do is what the summary call can do: Claude Code has no tools, no MCP servers and no hooks; Codex CLI has no tool-less mode, so it runs in a read-only sandbox with no network (it can read files, not change them or send anything); agy runs in plan mode with `--sandbox` (its changelog says plan mode does not edit or run commands; not tested here) and its conversation is deleted afterwards; OpenCode has the shell, edits, web fetches and other folders denied by rules and its session is deleted afterwards. No agent keeps a session. Sessions in excluded folders are never sent. It runs once a night, not while you are at work, (up to 300 exchanges) and with `eye clock detect`. `[clock] llm = false` stops it.

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
