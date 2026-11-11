---
name: eye
description: Find, resume and tidy up Codex CLI sessions with the eye board. Use when the user asks "where did we...", "which session...", "find the session...", "what was I doing...", "park this idea...", "mark this done...", or wants to know where a piece of work stands.
---

# eye

`eye` (the `agent-eye` binary; `eye` is its alias) indexes every Codex CLI session on this machine (title, folder, state, next step, outputs) and has an optional web board: `eye board on` turns it on (then `eye board` prints its URL), `eye board off` turns it off. It is off by default.

## Finding a session

The user has the terminal picker for themselves (`eye`, or Ctrl-G in the shell). For your own lookups use the JSON search below.

Run `eye find --json <words>` with the key words of the question (add `--all` to include archived and hidden sessions). Answer briefly with, for the best hit or two:

- title and folder (with branch if there is one)
- state: the column and the one-line state
- next step, if there is one
- the resume command, exactly as `resume_cmd` gives it

Do not paste the whole JSON. If nothing matches, say so and try other words once.

To see what a session was about before resuming it, `eye show <id> --conversation [-n 10]` prints the last messages (the user's prompts and Codex's replies), and `eye show <id>` includes the tokens it used. `eye resume <id> --fork --dry-run` prints the command that resumes it as a new branching session (`codex fork`).

## Parking an idea

`eye idea add "<title>" [--notes "<text>"] [--prompt "<text>"]` stores an idea in the Ideas column. `--notes` is for the user and is never sent to Codex; `--prompt` is the first message the new session gets when the idea starts (without one, Codex opens in the folder and waits for the user). Write the prompt as a clear task for a fresh session that knows nothing of this one. The folder is the current working directory unless `--dir <path>` is given. `eye idea edit <n> [--title] [--notes] [--prompt] [--dir]` changes one (an empty value clears notes or prompt). `eye idea ls` lists them. Starting one is done from the board.

## Marking a session

`eye mark --current <status> [--next "<text>"]` marks the session you are running in. Status is one of `done`, `paused`, `waiting`, `needs-you`, `archive`, `auto` (`auto` hands the card back to eye). For another session: `eye mark <id> <status>`.

## Checking on something later

`eye remind <id> <when> [--note "<text>"]` brings a session (or an idea, as `idea:<n>`) back to Needs you at a time, for a reply that may not come or an idea to look at by a day. `<when>` is `tomorrow`, `mon`..`sun`, `30m`, `4h`, `3d`, `1w`, `2026-11-20` or `"2026-11-20 14:00"` (a day alone means 9:00). `eye remind <id> --clear` removes it and `eye remind ls` lists them. Everything stays on the machine; a desktop notice for a due one is off unless the user turned it on (`eye remind notify on`). Use it only when the user asks.

Confirm in one line what you changed.
