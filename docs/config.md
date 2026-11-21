# Configuration

Settings live in `~/.agent-eye/config.toml` (or `$EYE_HOME/config.toml`). eye writes the defaults on
first run and reads the file on every start; the file is private to you (mode 0600). A key you leave
out keeps its default, so an old file keeps working. The board and the daemon read the file when
they start: after a change, `eye board off && eye board on` or restart the daemon.

## `[agents]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `["claude"]`; `eye install` sets it to the agents it finds | The agents eye follows, in order: `"claude"` (Claude Code), `"codex"` (Codex CLI), `"agy"` (Antigravity CLI), `"opencode"` (OpenCode). The first is the default agent (new ideas start in it, summaries are written by it). A config that has no `enabled` key means Claude Code only, so older files keep working; `install` fills the key in from what is on the machine, and mentions an agent it finds that the key leaves out. `[]` turns every agent off: eye still runs, with an empty board. An id nobody registered is reported by `eye doctor`. |
| `[agents.<id>] dir` | the agent's usual folder | Where that agent keeps its files, for example `[agents.claude] dir = "/Volumes/work/claude"`. A leading `~` is your home folder. For Claude Code the environment variable `EYE_CLAUDE_DIR` beats this key. For Codex CLI this key beats `CODEX_HOME`, which beats `~/.codex`. For Antigravity CLI the folder is `~/.gemini/antigravity-cli`. For OpenCode this key beats `$XDG_DATA_HOME/opencode`, which beats `~/.local/share/opencode`. |

Sessions and ideas of an agent that is not enabled are left out of every list. They stay in the
database and come back when the agent is enabled again.

## `[summary]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Write the state and next step of quiet sessions. `false` turns summaries off completely; nothing is sent anywhere. |
| `agent` | `""` | The agent that writes summaries, for example `"codex"`. Empty: the first enabled agent. It must be enabled. The summaries count against that agent's plan. |
| `model` | `""` | The model name handed to that agent. Empty: the agent's own cheap model, `haiku` for Claude Code and `gpt-6-luna` for Codex CLI. Model names belong to an agent, so change this when you change `agent`; an account that has no `gpt-6-luna` needs a model it has here. |
| `quiet_minutes` | `10` | A session is summarized after this many minutes without activity (or when it ended). |
| `concurrency` | `2` | How many summaries run at the same time. |
| `auto_days` | `3` | The daemon only summarizes sessions that were active within this many days (`eye summarize` can do older ones). |

## `[user]`

| Key | Default | Meaning |
|---|---|---|
| `name` | `""` | The name older summaries used for you. A stored next line that starts with `<name>:` is shown as `You:`. New summaries already say `You`. |

## `[board]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Serve the web board from the daemon. `eye board on` / `eye board off` change it. |
| `port` | `7420` | The board listens on `127.0.0.1:<port>` only. |
| `resume` | `"auto"` | How Resume and Start open a session. `"auto"`: on macOS an iTerm2 tab when iTerm2 is installed (`/Applications/iTerm.app` or `~/Applications/iTerm.app`), else a Terminal window; on Linux a new terminal window when `DISPLAY` or `WAYLAND_DISPLAY` is set, else the command is copied. `"iterm"`: an iTerm2 tab (macOS only; copies elsewhere). `"terminal"`: a new terminal window: on macOS eye writes the command into a `.command` file in `~/.agent-eye/tmp/` (removed after a day) and opens it, which runs in the app macOS uses for `.command` files, Terminal.app unless you changed it; on Linux the first of `xdg-terminal-exec`, `x-terminal-emulator` and `$TERMINAL` runs `sh -c '<command>'` (this one does not ask for a display). `"copy"`: copies the command. Where `"auto"` or `"terminal"` finds nothing to open, the command is copied. Any other value means `"auto"`. The board shows what a click will do (`ITERM`, `TERMINAL` or `COPY`, with the Linux terminal's name). Copying uses `pbcopy` on macOS and the first of `wl-copy`, `xclip` and `xsel` that works on Linux; with none, the command is shown instead. A config written before `"auto"` with `"iterm"` or `"copy"` keeps working as it was. |
| `done_days` | `7` | Done sessions idle for longer leave the board for the archive view. `0` keeps them. The picker and `eye ls` are not affected. |

## `[remind]`

| Key | Default | Meaning |
|---|---|---|
| `notify` | `false` | Off until you turn it on (`eye remind notify on`, the switch in the picker's Ctrl-R input or on the board); a config written before the key existed means off. When on and a "check on this" reminder (`eye remind`, the board's REMIND ME, Ctrl-R in the picker) falls due, the daemon shows one desktop notice for it: `osascript` on macOS, `notify-send` on Linux when it is installed, otherwise nothing. The title and the text are passed to the program as arguments, never written into a script. It never leaves the machine. `false` shows none; the card is still in Needs you, and a reminder that falls due while it is off is not announced when you turn it on later. The daemon reads the key again on each pass, so a change counts at once, without a restart. |

Why macOS once showed a notice without asking: `osascript` posts it as "Script Editor", an app macOS lets show notices without a permission prompt, so no prompt ever appeared for eye. To silence a notice you do not want, set `notify = false` here, or turn Script Editor off in System Settings > Notifications.

## `[night]`

Runs that start by themselves inside a time window. Off until you switch it on. See `night.md`.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Nothing runs unless this is `true` and a run is planned. `eye night on\|off` and the Tonight list on the board change it. |
| `window` | `"01:00-07:00"` | Local time, `HH:MM-HH:MM`. New runs start only inside it; one already going may finish. An end before the start crosses midnight (`23:00-05:00`). `eye night window` and the board change it. |
| `quiet_minutes` | `20` | A run starts only when no session you work in (live, interactive, not a night run) has been busy or touched for this long. |
| `parallel` | `2` | How many runs go at once (1 to 8). |
| `isolation` | `"worktree"` | How a run keeps your folder safe: `"worktree"` (a git worktree of the repo; fast; a command that builds a path in an unusual way can add a new top-level file in your folder) or `"copy"` (a separate local clone; your folder is locked completely). `eye night add --isolation` and the board's **Work in** choice override it per run. A plain folder is always a copy. See `night.md`. |
| `claude_permission` | `"auto"` | Claude's permission mode for night runs: `auto`, or `acceptEdits` (then only the build and test commands in `night.md` are allowed). |
| `claude_model` | `""` | A model for Claude night runs (an alias such as `haiku`); empty is Claude's default. |
| `codex_model` | `""` | A model for Codex night runs; empty is Codex's default. |
| `opencode_model` | `""` | A model (`provider/model`) for OpenCode night runs; empty is OpenCode's default. |
| `network` | the package registries | The only hosts a sandboxed Claude night run may reach from Bash: `registry.npmjs.org`, `pypi.org`, `files.pythonhosted.org`, `crates.io`, `index.crates.io`, `static.crates.io`, `proxy.golang.org`. Setting it replaces the list. `github.com`, `gitlab.com` and `bitbucket.org` are always refused. Not applied without the sandbox. |
| `agy_model` | `""` | A model for Antigravity (agy) night runs; empty is agy's default. |
| `allow_unsandboxed` | `false` | Antigravity and OpenCode night runs are not started without it: eye could not verify that `agy --sandbox` keeps a run in its folder, and OpenCode has no sandbox (see `night.md`). Claude night runs need Claude Code's sandbox (built in on macOS; `bubblewrap` and `socat` on Linux) and refuse to start without it. `true` starts them anyway; the deny rules stay, Bash is then not confined. See `night.md`. |

## `[loose]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Look for what sessions left behind: servers and shells still running, containers, git worktrees in a scratchpad or with no folder, commits on no remote, modified files nobody committed. On by default because finding them only reads. The daemon looks every 5 minutes and shortly after a session stops or ends (at most once in 20 seconds); `eye loose` looks at once. `false` stops the daemon's scans, hides the card chip, the panel section and the picker line, and makes `eye loose` say it is off. Stored results stay until the next scan that is allowed. |
| `days` | `30` | Only the folders and repos of sessions active in the last this many days are looked at. A scan reads `ps` and `lsof` (macOS) or `/proc` (Linux), `docker ps` when docker is installed (2 s limit) and a few read-only git commands per repo, each with a time limit. |

## `[hide]`

Throw-away sessions ("/clear", "exit", "cd ..") are left out of the board, search and lists. See
`api.md` ("Hidden sessions") for the exact rule.

| Key | Default | Meaning |
|---|---|---|
| `trivial` | `true` | Hide sessions that look like throw-away commands. |
| `max_prompts` | `1` | A trivial session has at most this many prompts. |
| `max_reply_chars` | `200` | ... and a last reply no longer than this. |
| `commands` | `["exit", "quit", "resume", "usage", "clear", "cd", "help"]` | First words that mark a prompt as a command. |

## `[exclude]`

| Key | Default | Meaning |
|---|---|---|
| `paths` | `[]` | Folder prefixes eye ignores completely: sessions started there are never indexed. |

## `[outputs]`

| Key | Default | Meaning |
|---|---|---|
| `jira_projects` | `[]` | Project prefixes that count as Jira projects, e.g. `["PROJ", "ACME"]`. Empty: eye learns them from links and branch names. |
| `jira_url` | `""` | Your Jira site, e.g. `"https://acme.atlassian.net"`, for keys that were never seen in a link. |

## `[backup]`

Snapshots of the database (see `backups.md`). Every key is optional.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Make a snapshot every day (the daemon does it) and before a schema upgrade. `false` stops both; `eye backup` by hand still works and says that automatic ones are off. |
| `keep_daily` | `7` | How many daily snapshots to keep. |
| `keep_weekly` | `4` | How many weeks keep one snapshot, the newest daily of that week. |
| `mirror` | `""` | An absolute path (or one that starts with `~`) to a folder that **already exists** (an external disk, a synced folder): each snapshot is copied there too, with the same rotation. A folder that is not there is skipped, never created, and `eye doctor` says so. A leading `~` is your home folder. Empty: off. |
| `mirror_archive` | `false` | Also copy the transcript archive (`~/.agent-eye/archive/`) into `<mirror>/archive`: only files whose size or time changed, and nothing is ever deleted there. |

## `[usage]`

The cost estimate (see `usage.md`). Every key is optional.

| Key | Default | Meaning |
|---|---|---|
| `[usage.prices."<model>"]` | the built-in table | Prices in USD per million tokens for a model, to add one or override a built-in price: `input`, `output`, `cache_write`, `cache_read` (each defaults to 0). The model id matches with its date and `[1m]` tag left off. A model named here is priced from this table alone; a model in neither table has no cost, only tokens. |

## Environment

| Variable | Meaning |
|---|---|
| `EYE_HOME` | eye's own folder (default `~/.agent-eye`). |
| `EYE_CLAUDE_DIR` | Claude Code's folder (default `~/.claude`). |
| `EYE_CLAUDE_BIN` | The program to run instead of `claude` (used by tests). |
| `CODEX_HOME` | Codex CLI's folder (default `~/.codex`), as Codex itself reads it. |
| `EYE_CODEX_BIN` | The program to run instead of `codex` (used by tests). |
| `EYE_AGY_BIN`, `EYE_OPENCODE_BIN` | The program to run instead of `agy` or `opencode` (used by tests). |
| `EYE_FACE_DIR` | Serve the board's web files from this folder (development). |
| `NO_COLOR` | Turn colours off in the picker. |
| `EYE_PICK_TIMING` | Show the search time in the picker's corner. |
