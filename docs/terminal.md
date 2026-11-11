# Terminal picker

The package and binary are `agent-eye`; `eye` is its alias, and the examples use it. Data lives in `~/.agent-eye`.

`eye` with no arguments (or `eye pick [words]`) opens a picker on the terminal. Type words, the best session is on top; Enter lands you in it. Sessions of every enabled agent are listed together (Claude Code by default; see `agents.md`); a session of an agent that is switched off in `[agents] enabled` is left out.

- Words are AND-ed prefix matches over titles, prompts, replies, outputs (PR URLs, Jira keys) and folders, merged with a fuzzy match on titles. A title hit ranks first, then body hits, then fuzzy matches; the archive comes last. Inside each group recent and active sessions win: last activity within 7 days, then within 30 days, then older; then status (needs you first); then how close the match is; then newest. `eye find` and board search use the same order.
- Empty query: Needs you, Running, Waiting on others, Ideas, Paused, Done. Hidden sessions and the archive are left out.
- Each row: status mark (red ● needs you, green ▶ running, blue ◷ waiting, ✦ idea, dim ‖ paused, dim ✓ done), title, project (last folder), age.
- The preview (right when wide, below when narrow) shows state, next step and who acts (the label says who, so the line does not repeat its owner: `Next (you) review the PR`), topics, outputs, sub-agents, tokens used ("Tokens 2.3M (sub-agents 410k)") and the resume command. The preview and the conversation view name the session's agent ("Claude") where they refer to it.
- A rescan (about 20-40 ms) runs before the screen opens, so results are fresh without the daemon.

## Keys

| Key | Does |
|---|---|
| Up / Down, Ctrl-P / Ctrl-N, PgUp / PgDn | move |
| Enter | resume the session (restores the transcript from the archive first if it is gone); on an idea, starts it here |
| Ctrl-F | fork the session: resume it as a new session that branches from it (for Claude Code `claude --resume <id> --fork-session`); the original stays as it is. No open-elsewhere question, forking a live session is fine. An agent that cannot fork says so |
| Ctrl-T | switch the preview between the summary and the conversation of the selected session (your prompts and the agent's replies, newest at the bottom; it is read from the transcript or the archive copy when you open it) |
| Shift-Up / Shift-Down | scroll the conversation view |
| Ctrl-Y | copy the resume command (`pbcopy` on macOS; `wl-copy`, `xclip` or `xsel` on Linux). With no clipboard tool the command is shown instead |
| Tab | hide or show the preview |
| Ctrl-X | hide the session (a useless one); on a hidden one, show it again |
| Ctrl-A | include hidden sessions and the archive |
| Ctrl-U / Ctrl-W | clear the query / delete a word |
| Esc, Ctrl-C | quit, prints nothing |

Enter on a session leaves the screen and runs the session's agent in the session's folder (for Claude Code `claude --resume <id>`), replacing eye in this terminal. With `--print` it writes the shell line (`cd '<dir>' && claude --resume <id>`) to stdout instead. The folder and every argument are quoted, and an id that is not plain letters, digits, `-`, `_` or `.` (or starts with `-` or `.`) is refused, and so is a folder with a control character in its name. Starting an idea records the start, as `eye idea start` does, and sends the idea's prompt (never its notes); an idea without a prompt opens the agent waiting for you. An idea whose folder does not exist shows `+` before its project in the list and a line in the preview; Enter asks first (Enter again creates the folder and starts, Esc cancels). Eye creates folders only inside your home folder; for any other missing folder, or a file in the way, Enter says why and starts nothing. Ctrl-F leaves the same way with the agent's fork command (or prints that line with `--print`). A session that is open in another terminal asks first: Enter again resumes anyway, Esc cancels.

## Shell key

Add one line to `~/.zshrc` (or `~/.bashrc` with `bash`):

    eval "$(eye init zsh)"

To keep the shell starting cleanly if eye is ever uninstalled, guard it:

    command -v eye >/dev/null && eval "$(eye init zsh)"

Ctrl-G then opens the picker with the current command line as the start query. The chosen command lands on the command line (nothing runs until you press Enter, and it goes into your normal history). Esc leaves the line untouched. Another key: `eye init zsh --key ctrl-o`. The widget calls the program under the name it was invoked as. `eye doctor` prints the line if it is missing.

## Without a screen

- `eye pick --filter "words"` prints the ranked list (`--limit`, `--all`).
- `eye pick --filter "words" --first --print` prints the first hit's command; without `--print` it runs it.
- `eye pick --filter "words" --first --fork [--print]` forks the first hit instead.
- `eye show <id> --conversation [-n 10]` prints the last messages of a session (`--json` for the API shape); `eye resume <id> --fork` prints the fork command (and opens it like `eye resume`; add `--dry-run` to only print).
- `eye idea start <id>` on an idea whose folder does not exist asks `Folder <path> does not exist. Create it? [y/N]` in a terminal; `--create-folder` makes it without asking (for scripts), and without a terminal and without the flag it stops with a message that names the flag. `--dry-run` creates nothing.
- `eye idea add` and `eye idea edit` take `--agent <id>` to start an idea in a particular agent (default: the first enabled one).
- Without a terminal, `eye` prints the list like `eye ls`.

## Config and environment

- Hidden sessions follow `[hide]` in `config.toml`; which agents are followed is `[agents] enabled`. `docs/config.md` lists every key.
- The web board is optional and off by default (`[board] enabled`). `eye board on` turns it on, `eye board off` turns it off; the picker never needs it.
- `NO_COLOR` turns colours off. `EYE_PICK_TIMING=1` shows the search time in the corner.
- `EYE_CLAUDE_BIN` replaces the `claude` binary that Enter, Ctrl-F and the summaries run (used by tests). `EYE_CLAUDE_DIR` points eye at another Claude Code folder, `EYE_HOME` at another eye home.
- `--no-scan` skips the rescan.
