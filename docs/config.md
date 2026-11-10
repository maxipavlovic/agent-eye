# Configuration

Settings live in `~/.agent-eye/config.toml` (or `$EYE_HOME/config.toml`). eye writes the defaults on
first run and reads the file on every start; the file is private to you (mode 0600). A key you leave
out keeps its default, so an old file keeps working. The board and the daemon read the file when
they start: after a change, `eye board off && eye board on` or restart the daemon.

## `[agents]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `["claude"]`; `eye install` sets it to the agents it finds | The agents eye follows, in order: `"claude"` (Claude Code), `"codex"` (Codex CLI). The first is the default agent (new ideas start in it, summaries are written by it). A config that has no `enabled` key means Claude Code only, so older files keep working; `install` fills the key in from what is on the machine, and mentions an agent it finds that the key leaves out. `[]` turns every agent off: eye still runs, with an empty board. An id nobody registered is reported by `eye doctor`. |
| `[agents.<id>] dir` | the agent's usual folder | Where that agent keeps its files, for example `[agents.claude] dir = "/Volumes/work/claude"`. A leading `~` is your home folder. For Claude Code the environment variable `EYE_CLAUDE_DIR` beats this key. For Codex CLI this key beats `CODEX_HOME`, which beats `~/.codex`. |

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

## Environment

| Variable | Meaning |
|---|---|
| `EYE_HOME` | eye's own folder (default `~/.agent-eye`). |
| `EYE_CLAUDE_DIR` | Claude Code's folder (default `~/.claude`). |
| `EYE_CLAUDE_BIN` | The program to run instead of `claude` (used by tests). |
| `CODEX_HOME` | Codex CLI's folder (default `~/.codex`), as Codex itself reads it. |
| `EYE_CODEX_BIN` | The program to run instead of `codex` (used by tests). |
| `EYE_FACE_DIR` | Serve the board's web files from this folder (development). |
| `NO_COLOR` | Turn colours off in the picker. |
| `EYE_PICK_TIMING` | Show the search time in the picker's corner. |
