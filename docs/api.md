# eye board API

JSON over HTTP on `127.0.0.1:<port>` (default 7420, `[board] port` in config). The board shows sessions of every enabled agent (Claude Code by default; see `agents.md`). The web board is optional and off by default: `eye board on` turns it on (`[board] enabled = true` in config; the daemon then serves it), `eye board off` turns it off, and `eye serve` runs it once in the foreground. The face in `face/` uses only these routes; a different front end can do the same.

## Access

- The token is in `$EYE_HOME/board.token` (default `~/.agent-eye`) (created once, mode 0600). `eye board` (when the board is on) prints `http://127.0.0.1:<port>/?t=<token>`.
- `GET /?t=<token>` sets the cookie `eye_token` (HttpOnly, SameSite=Strict, Path=/) and redirects to `/`, keeping any other query parameters (`/?t=<token>&look=ink` → `/?look=ink`).
- `EYE_FACE_DIR=<dir>` makes the server read the face from that folder on every request (for development: `npm run watch` in `face/`). If it has no `index.html`, `/` serves a plain page saying the board was not built in.
- Every other request needs that cookie or the header `X-Eye-Token: <token>`; otherwise `401`.
- `Host` must be `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>` (any case); otherwise `403`, whatever the token says. This stops a web page on another name that was pointed at 127.0.0.1 (DNS rebinding).
- A non-GET request must carry an `Origin` header that is same-origin (`http://127.0.0.1:<port>`, `http://localhost:<port>` or `http://[::1]:<port>`); otherwise `403`.
- Errors are `{"error": "<message>"}` with a 4xx/5xx status. Times are Unix milliseconds.

## Routes

| Route | Does |
|---|---|
| `GET /` | the board (the TypeScript face built from `face/`, embedded in the binary). Without an embedded face it serves a plain page saying the board was not built into this binary (run `npm run build` in `face/` and rebuild). |
| `GET /assets/*` and other face files | the face's built files. Hashed names under `/assets/` are `immutable`; the HTML is `no-cache`. Responses carry a CSP: `'self'` only, plus `data:` images and fonts and inline styles. Unknown paths are 404. |
| `GET /api/board[?all=1]` | `{columns:[{id,title,cards:[Card]}], archive_count, generated_at}`. Columns, in order: `ideas, running, needs_you, waiting, paused, done`. Archive is not included. Cards are newest activity first. A `done` session idle for more than `[board] done_days` (default 7, 0 = no limit) leaves the board and is counted in `archive_count`. |
| `GET /api/prefs`, `POST /api/prefs` | Board settings shared by every browser and the installed app (`eye.theme`, `eye.accent`, `eye.ink.layout`, `eye.ink.fold`), kept in `~/.agent-eye/ui.json`. POST merges `{key: "value"}`; other keys are refused. `eye.accent` is the ink look's signal colour, chosen with the swatches in the board's header: `vermilion` (the default), `cobalt`, `teal`, `moss`, `amber`, `magenta`, `violet` or `graphite`; a missing or unknown value is vermilion. Each preset has a light and a dark pair, set as `data-accent` on `<html>`. The terminal picker keeps your terminal's colours. |
| `GET /api/archive?offset=0&limit=50[&all=1]` | `{cards:[Card], total, offset}`; archived and history-only sessions, plus `done` sessions that aged off the board (`[board] done_days`), newest first (limit 1..200) |
| `GET /api/cards/:id` | Detail Card. `:id` is a session id, a unique prefix of 6+ chars, or `idea:<n>`. `resume_mode` (`iterm`, `terminal` or `copy`) and `resume_terminal` (the Linux terminal's name or null) say what Resume will do on this machine. |
| `POST /api/cards/:id/column` | body `{column}`: a column id (`running`, `needs_you`, `waiting`, `paused`, `done`), `archive` (archives the session), or `"auto"` (clears the manual column and un-archives); `ideas` is a 400. Returns the Card. Moving a session to a column sets `column_manual` until the session has new activity. Idea cards accept only `archive`. |
| `POST /api/cards/:id/hidden` | body `{hidden: true\|false}`: same as `eye hide` / `eye unhide` (sets the manual override in `session.hidden`; see Hidden sessions). The session leaves `/api/board` on the next fetch (or comes back). Returns the Card. Idea cards are a 400. |
| `POST /api/cards/:id/resume` | body `{dry_run, fork}` (`dry_run` defaults to true, `fork` to false). Returns `{dry_run, mode, terminal, command, applescript}`. With `fork:true` the command resumes the session as a new one that branches from it (for Claude Code `cd '<cwd>' && claude --resume <id> --fork-session`), the original stays untouched (a live session can be forked). An agent that cannot fork gives a 400. The command is built by the session's own agent; a session of an agent that is switched off gives a 400. With `dry_run:false` it also puts the transcript back from eye's archive when the agent has deleted it, then runs the plan: opens an iTerm2 tab (`mode: iterm`, macOS only), opens a new terminal window (`terminal`) or copies the command (`copy`), and adds `note`. `mode` is what `[board] resume` comes to on this machine, worked out once per request (`auto` is never returned): the same answer as `resume_mode` in the card detail. `terminal` is the name of the Linux terminal program that opens the window (`xdg-terminal-exec`, a name like `gnome-terminal`, `$TERMINAL`), or null. When no clipboard tool took the command, `note` says so and holds the command (the background process may have no display). `applescript` is set in `iterm` mode only. A failure, a terminal that does not open included, is a 400 with the reason. |
| `GET /api/cards/:id/conversation?limit=10&before=<i>` | The conversation of a session, read from its transcript by its agent (or eye's archive copy when the transcript is gone): `{messages:[{role,text,ts,tools,truncated}], more, next_before}`. Messages are numbered 0..N-1, oldest first. The response holds the `limit` messages just before index `before` (default N, so the newest), still oldest first; `limit` defaults to 10, max 50. `more` says older messages exist; `next_before` is the `before` to ask for next (null when `more` is false). `role` is `user` (a real prompt: paste tags removed, slash commands, tool results and injected lines left out) or `assistant` (the text blocks of the agent's replies between two prompts, joined with a blank line; `text` can be empty when the turn was only tool calls); `tools` = tool calls the agent made in that turn (0 for user messages); `ts` = Unix ms or null. `text` is cut at 4000 characters (`truncated:true`) and secrets are masked. Sub-agent lines are left out. 404: unknown id, or no transcript and no archive copy; 400: an idea. |
| `GET /api/search?q=words&limit=30[&all=1]` | `{results:[{card, snippet, score}]}`; searches everything, archive included. Ordered like the picker (see terminal.md): archive last, title hits before body hits, then recent and active first. |
| `GET /api/agents` | `{agents: [{id, name, short, default}]}`: the enabled agents in config order, e.g. `{id:"claude", name:"Claude Code", short:"Claude", default:true}`. The first one is the default. Empty when none is enabled. |
| `GET /api/ideas` | `{ideas:[Idea]}` (not archived, and not started, or started but its session never showed up) |
| `POST /api/ideas` | body `{title, notes?, prompt?, folder?, agent?}` (a leading `~` is expanded). `notes` are never sent to the agent; `prompt` is the first message when the idea starts; `agent` is the id of an enabled agent (empty or left out: the default agent; an agent that is not enabled is a 400). Returns the idea Card. |
| `POST /api/ideas/:id` | Edit: body with any of `{title, notes, prompt, folder, agent}`; a field left out stays, an empty string clears notes, prompt, folder or agent (an empty title is a 400). Returns the idea Card. 404: no such idea. |
| `GET /api/folder?path=<p>` | What a typed folder is, for the idea form: `{state, creatable, note}`. `state` is `ok` (a folder is there), `missing`, `not_a_folder` (a file is in the way) or `unusable` (empty, not an absolute path, a control character, not readable); `creatable` is true only for a missing folder that eye may make; `note` is one plain line, null when `state` is `ok`. A leading `~` is the home folder. Read-only: nothing is created. |
| `POST /api/ideas/:id/start` | body `{background, dry_run, create_folder}` (dry_run defaults to true; `create_folder` to false). Returns `{dry_run, mode, terminal, command, applescript, session_id, job, note, creates_folder}` (`mode` and `terminal` as for resume; a background start reports `copy`). The command is the idea's agent's (the default agent when none is set) and sends the idea's `prompt` as the first message; without one, foreground opens the agent with nothing sent and background is a 400. `session_id` is null when the agent does not let eye choose it. With `dry_run:false`: foreground stores `session_id` on the idea and opens iTerm2 or a terminal window (or copies, and `note` says which); until that session shows up the idea stays in Ideas and a new start reuses the same id; background runs the agent's background command (`claude --bg`) and returns its `job` short id (`session_id` is null until the daemon links the job; the idea leaves the Ideas column then). The agent's own message (e.g. Claude Code's "Workspace not trusted. Run `claude` in <dir> once...") comes back as a 400 `error`. A folder that does not exist is never started in: without `create_folder` the answer is 409 `{error, code: "folder_missing"}`; with it the start makes the folder (and the folders above it) first and `creates_folder` is true (a dry run only says so and creates nothing). Eye makes folders only inside the account's home folder, never through a `..` or a link that leaves it; any other missing folder, a file in the way or an unusable name is a 400 whose `error` says why, even with `create_folder`. |
| `GET /api/events` | Server-sent events. Event `board-changed` when the database changed (checked every 2 s via `PRAGMA data_version`). Reload `/api/board` on it. |

## Config

`docs/config.md` lists every key. The ones that change what the API returns:

```
[board]
enabled = false
port = 7420
resume = "auto"    # or "iterm", "terminal", "copy"
done_days = 7      # Done sessions older than this leave the board for the archive view; 0 = keep them

[agents]
enabled = ["claude"]   # the agents eye follows; the first is the default

[user]
name = ""              # a stored next line "<name>: ..." is served as "You: ..."
```

`done_days` only affects `/api/board` and `/api/archive`; the picker, `eye ls` and the column rules are unchanged. Sessions and ideas of an agent that is not in `[agents] enabled` are left out of every list (they stay in the database and come back when the agent is enabled again).

Jira keys in outputs get a `url` of `https://<site>/browse/<KEY>`. The site is the one that project's keys were seen linked on in your transcripts (`...atlassian.net/browse/KEY-1`); `[outputs] jira_url` covers projects never seen in a link.

## Hidden sessions

Throw-away sessions ("/clear", "exit", "cd ..") are left out of `/api/board` (and its `archive_count`), `/api/archive`, `/api/search`, `eye find` and `eye ls`. Add `?all=1` (CLI: `--all`) to include them. `GET /api/cards/:id` still returns a hidden session. Summaries skip hidden sessions; the archive copy of the transcript still happens.

A session is trivial when all of these hold: `prompt_count <= max_prompts`, no outputs and no sub-agents, the last reply is at most `max_reply_chars`, it is not busy, and the first prompt starts with `/`, starts with a word from `commands` (case-insensitive), or is a single word of at most 10 characters. Config, on by default, in `config.toml`:

```
[hide]
trivial = true
max_prompts = 1
max_reply_chars = 200
commands = ["exit", "quit", "resume", "usage", "clear", "cd", "help"]
```

`eye hide <id>` and `eye unhide <id>` set a manual override stored in `session.hidden` (1 hidden, 0 shown, NULL the rule decides). An unhide beats the rule; a hide beats a session that looks real. `eye doctor` prints how many sessions are hidden.

## Card

```
{id, agent, agent_name, title, folder, repo, branch, column, auto_column, manual, last_activity_at, started_at,
 state, state_source, next, ai_status, recap, live_status, kind, streams, outputs, output_counts,
 topics, topics_more, resume_cmd, resumable, parent_id, child_count, child_done, prompt_count, tokens}
```

- `agent`: the id of the agent that ran the session (`claude`); `agent_name`: its short name for sentences (`Claude`; the id when the agent is switched off). Ideas carry the agent they start in, or null for the default agent.
- `column`: where it is shown. `auto_column`: where the rules would put it. `manual`: a hand-placed column is in effect.
- `next`: who acts next and what, `<owner>: <action>`, e.g. `You: review the PR`. The owner is `You` for the user, the agent's short name for the agent (`Claude: fix the failing test`) or another person's name. Lines stored with the user's configured name (`[user] name`) are served as `You:`. While a session is open, not working and waiting on you (a PermissionRequest, a permission Notification, or a reply ending in a question; the "waiting for your input" notification that follows every finished reply does not count), it is what the agent's hook says is asked instead: `You: answer: <question>`, `You: allow Bash: <command>` (secrets masked, at most 240 characters), `You: approve or change the plan`. Otherwise it is the written summary's next step, or null.
- `blocked`: true while a prompt in the open session (a permission, a question dialog, a plan to approve) stops the agent mid-task until you answer it; the board makes its YOU chip blink. A finished reply that ends in a question waits on you but is not blocked.
- `state_source`: `summary` (the written summary, whichever agent wrote it), `recap` (the agent's own recap, first 220 chars, used until a summary exists), `idea` (notes) or `none`.
- `folder`, `repo` have `~` for the home directory. `branch` is null for detached HEAD.
- `kind`: `interactive`, `bg`, `history_only`, or `idea`. `resumable` is false for history-only sessions and ideas.
- `child_count`, `child_done`: sub-agents and background children. Board cards do not list them.
- `topics`: `[{key,title,status,next}]`, at most 5 on board cards, `topics_more` = how many were cut. Status is `done|active|paused|waiting_on_others|needs_you`.
- Board cards: `streams` holds topics only, and `outputs` is the card face selection, ordered PR, artifact, Jira (at most 3; keys that also appear in the title, branch or a PR first), one folder. `output_counts` gives the true totals per kind.
- Detail card (`GET /api/cards/:id`): all `streams` (`{kind,title,key,status,detail,link}`), all `outputs` (`{kind,value,url,title}`; `title` is set for artifacts when one is known, else null), all `topics`, plus `prompts` (up to the last 10 readable ones, oldest first, from the search index; `<pasted_content>` tags are removed, paste-only prompts read "[pasted] …", slash commands are skipped), `first_prompt` (the first readable prompt), `last_assistant_text`, `resume_mode`, and `idea` (`{id, title, notes, prompt}` of the idea the session was started from, or null). Idea detail adds `notes` and `prompt`.
- `resume_cmd`: the command that resumes the session, built by its agent, e.g. `cd '<cwd>' && claude --resume <id>`. Null when the folder is unknown, the id is not one eye will put in a command, or the agent is switched off.
- `tokens`: `{input, output, cache_write, cache_read, total, subagents_total}` or null (ideas, and sessions whose transcript was not read yet). Counted from the `usage` of the assistant messages in the transcript, each message once (Claude Code writes one line per content block with the same usage; another agent's adapter counts its own way). `subagents_total` is the sum of the four counts over the session's sub-agent transcripts; `total` = input + output + cache_write + cache_read of the main transcript + `subagents_total`. A full rescan (`eye scan --full`) recomputes it.

## Idea

`{id, agent, title, notes, prompt, folder, created_at, from_session_id, session_id, sort, archived}`; the idea Card has `id: "idea:<id>"`, `idea_id`, `state` = notes, `has_prompt`, and, when the idea has a folder, `folder_state` (`ok`, `missing`, `not_a_folder` or `unusable`, as in `/api/folder`), `folder_creatable` and `folder_note`.
