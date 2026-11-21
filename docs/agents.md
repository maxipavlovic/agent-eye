# Agents

eye follows coding agents through adapters. An adapter is one Rust type that implements the `Agent`
trait in `src/agents/mod.rs`. Everything else (the scanner, database, picker, board, daemon, archive
and search) talks to that trait and knows nothing about a particular agent. Claude Code (the default),
Codex CLI, Antigravity CLI (`agy`) and OpenCode are the real adapters; `src/agents/claude/` and
`src/agents/codex/` are the worked examples, `src/agents/agy/` and `src/agents/opencode/` show a session
that is a folder with a database and one that is a row in a shared database.

## What an adapter provides

The required methods say where an agent's sessions are and how to read them. Every optional method
has a default that means "this agent does not offer that". The defaults for reading and archiving
are the ones for a session that is one text file which only grows (see "Storage styles").

| Area | Methods | Notes |
|---|---|---|
| Identity | `id`, `name`, `short_name`, `detect` | `id` is stored in the database and used in config and the API (`"claude"`). `short_name` is used in sentences and as the owner in `next` lines (`Claude: fix the test`). `detect` also decides whether `eye install` turns the agent on. |
| Sessions on disk | `transcripts`, `parse_transcript`, `owns_transcript` | `transcripts` lists what holds each session (a file, a folder, a database) with a change stamp. `parse_transcript(transcript, opts)` reads a session, all of it or what is new since the last read, and returns a `Delta` (see below). |
| Usage by day | `usage_msgs`, `Delta::usage_msgs`, `UsageRead::msgs`, `Delta::limits` | Optional (the default reports none). The token use of every assistant message with its time and model, which the core adds into days per model (`usage.md`); `Delta::limits` carries the plan limits a transcript reports, newest line wins. `usage_msgs(path)` reads a whole transcript (a session's or a sub-agent's) for the backfill and must add up to the same tokens as `parse_transcript`. |
| Sub-agents, tokens, history | `subagents`, `read_usage`, `history_source`, `read_history`, `parse_history_line`, `own_dirs` | Optional. Sub-agents become child streams on the card; `read_usage` counts their tokens; the history keeps prompts of sessions whose transcripts were deleted. `read_usage` and `read_history` continue from a marker like `parse_transcript` does. |
| Live state | `live_sessions`, `live_status_shows_prompts`, `jobs` | Optional. Which sessions are open now (busy, idle, waiting, with why it waits, its remote control link and the loops its live state shows) and which run in the background. `live_status_shows_prompts` defaults to yes; an agent whose live status stays `busy` while it waits on a permission prompt returns no, and then a prompt a hook recorded makes the session `waiting` instead of `busy` until a hook says it was answered. |
| Left behind | `scratch_roots` | Optional. Folders where the agent keeps one scratch folder per session (Claude Code: `<tmp>/claude-<uid>/<project>/<session-id>/`). A process, container or git worktree in one is the named session's, found by the session id in its path. Default: none; Codex has no scratchpad, so what it leaves is found by working folder and active hour only. |
| Hooks | `queue_line`, `wait_effect` | Optional. `queue_line` turns a hook payload into one short stored line; `wait_effect` says whether that event makes a session wait on you, and what it asks. A `Wait` with `only_if_prompt` set changes the wait only while it is a prompt (an event that shows a prompt was answered). |
| Text | `display_prompt`, `clean_recap` | Optional. `display_prompt` says how a stored prompt line is shown, or that it is not a prompt (a built-in command, a paste tag with nothing in it); the default only trims. `clean_recap` takes out what the agent adds to every recap; the scanner stores recaps through it. |
| Reading | `read_conversation`, `conversation_tail`, `ask`, `default_summary_model` | Conversation for the board and picker, the tail of a transcript for the summary excerpt, and a one-shot structured question to the agent (summaries). The first two get a path that exists: the session itself, or a temporary copy eye made from the archive. Without `ask` the agent cannot write summaries. `default_summary_model` is the model `ask` gets when `[summary] model` is empty. |
| Forget | `delete_transcript` | Optional (the default says the agent does not support it). Remove the agent's own copy of a session for `eye forget --transcript` and return what went, as lines for people. Check that the path is inside the agent's folder first. Claude Code: the transcript file and the folder of the same name (sub-agents). Codex: every rollout of the thread and its line in `session_index.jsonl`. agy: the conversation's `brain/<id>/` folder, its database with `-shm` and `-wal`, and its row in `conversation_summaries.db`. OpenCode: the session's rows (and its sub-agents' rows) in `opencode.db`, or its JSON files. |
| Archive | `transcript_stamp`, `transcript_exists`, `archive_name`, `archive`, `unarchive` | Optional. How a session is copied to eye's archive and put back; see below. The defaults make one zstd file per session. `transcript_exists` says whether the transcript path is there to read; the default asks the file system, an agent whose path names a session inside a database answers itself. |
| Commands | `executable`, `resume`, `fork`, `start`, `background_job_id` | Return an `Invocation` (program and arguments). eye adds the folder, quotes every argument and decides how to run it. `fork` is optional. `start` may return no session id when the agent cannot choose one; eye then links the idea to the first session of that agent that begins in its folder. |
| Setup | `preflight`, `install`, `uninstall`, `doctor`, `daemon_env`, `executable_dir` | Install hooks and a skill (or whatever the agent has), remove them again, and check that the agent's files still look as eye expects. `install` and `uninstall` return a `Report`: one row for the agent (`hooks added (6 events) to ~/.codex/hooks.json, skill added`), the file diff as that row's detail, and a `next` line when the user has to do something (Codex's `/hooks` trust). `install` must leave other tools' entries alone and must not create files for an agent that is not there. |

## Reading a session again and again

A `Transcript` is what holds one session: its `path` (a file, a folder or a database; only the
adapter looks inside), the project it belongs to, the session id, and a `stamp`, two numbers that
change whenever the session does. For a file they are its size and modification time. eye skips a
session whose stamp is the same as at the last read, without opening it.

When the stamp moved, `parse_transcript` is called with `opts.since`: the **marker** the adapter
returned last time and the stamp it had then. The marker is a string only the adapter understands
(for Claude Code, the byte offset after the last complete line). The adapter decides:

- It can go on from the marker: it reads only what is new and returns a `Delta` of that.
- It cannot (the session was rewritten, the marker means nothing any more): it reads everything
  and sets `replaces`. eye then throws away what it holds for the session and stores the new
  `Delta`, so nothing is counted twice.

Either way the `Delta` carries the new `marker`, and `bytes_read` for the scan statistics. When
`since` is empty (a first read, `eye scan --full`, a session that came back) the adapter reads
everything. `opts.carry` and `opts.last_msg_id` belong to the last read: when the adapter reads
everything again, it leaves them out. `read_usage` and `read_history` work the same way and return
a marker and `replaces` too.

The `Delta` (in `src/agents/types.rs`) holds the session id, folder, branch, first and last time,
prompt and reply counts and text for search, title candidates, outputs (pull requests, artifacts,
commits, folders, Jira keys), token use, and an opaque `carry` value that comes back with the next
read. Session ids must be unique across agents: they are the key of the `session` table.

## Work a session does on its own

An open session can have background tasks (a shell, an agent, a monitor it started and left running) and loops (it runs again by itself). No new column: such a session stays in Running with one line saying what, and a live `waiting` on a prompt puts it in Needs you with the reason. A transcript adapter reports the tasks and loops in `Delta::signals` (the whole current state after the read, kept between reads in `carry`); a live-state adapter reports the waiting reason, a remote control link and loops in `LiveSession`. The scanner stores them (`signals`, `live_wait`, `remote_url`, `live_loops`) and `activity.rs` decides what still counts: nothing for a session that is not open, a task older than 12 hours (its end was never seen), a recurring loop older than 7 days, a self-paced loop 15 minutes after its next wake-up passed without a new one.

| | Claude Code | Codex CLI |
|---|---|---|
| Background tasks | `Bash`, `Agent`/`Task` and `Monitor` calls whose result names a task id (`backgroundTaskId`, or `agentId` of an async agent; also a command the timeout moved to the background). They end with a `<task-notification>` for the id (a `queue-operation` line or a user message) or a `TaskStop`/`KillShell` call. | `exec_command` results that carry a `session_id` and no `exit_code` (or `Process running with session ID N`). They end with the `CommandExecution` item of that `process_id`, or a later result for the id with an `exit_code`. The id is only visible when the program printed the whole result; a process whose id was never shown is not found. The rollout has no start record, so the start is a heuristic. |
| Loops | `CronCreate` (until `CronDelete`) and `ScheduleWakeup` (until the next one replaces it, or `stop`). `/schedule` routines run in the cloud and are not seen. | `/goal` only: a thread with an `active` row in `goals_1.sqlite` (`thread_goals`) restarts turns when idle. The file is opened read-only, and a missing file or table is no goal. Queued messages, heartbeat turns and desktop-app automations are not read. |
| Waiting reason | `waitingFor` of the live file (`permission prompt`, `input needed`, ...). `dialog open` is a dialog the user opened, not an ask. | A permission prompt is not on disk; the hook says it (see above). |
| Remote control | `bridgeSessionId` of the live file, as `https://claude.ai/code/<id>` (only plain id characters are made into a link). | Nothing: it is per daemon, not per session. |

What was seen on real Claude Code 2.1.311 files: the shapes of background shells and agents and of their notices. `CronCreate`, `ScheduleWakeup`, `Monitor` and `TaskStop` results were not on this machine to read, so their ids and a monitor's notices follow the documented tool inputs; a `<task-notification>` whose status says it still runs does not end a task.

## Storage styles

**An append-only file** (Claude Code, Codex CLI: one JSON-lines file per session). The stamp is the
file's size and time and the marker is the byte offset. `agents::append_only` has what is needed:
`resume_offset(opts.since, stamp)` gives the offset to continue from, or none when the file shrank
or is older than at the last read (it was replaced), and `for_each_line` reads complete lines from
an offset. The default `read_history` uses it too. Archiving needs nothing more: the defaults copy
the one file with zstd.

**A file that is rewritten** (an agent that saves the whole session on each turn). There is no
offset to continue from. The stamp still says that the file changed. Let the marker say what was
read, for example the number of messages and a hash of them. If the file still starts with what was
read, return only the new messages; if not, read it all and set `replaces`. Reading everything every
time is also correct, only slower. Archiving is the default as well.

**A folder** (an agent that keeps a session in several files). Make the stamp from the whole
folder, for example the total size and the newest modification time of the files. The marker names
whatever the adapter needs to go on: a file and an offset in it, a count of files. Override
`archive` to write a folder of compressed files, or one tar.zst (`archive::compress_file` and
`decompress_file` do one file), and `unarchive` to make the folder again. `archive_name` is the
name of the copy, `<id>` for a folder. `transcript_stamp` must give the stamp of the folder, so that
eye copies it again when it changed.

**A SQLite database** (an agent with a database per session, or one database for all). Open it
read-only to read, and take care not to hold it open or locked: the agent writes to it. A rowid or
a version number makes a good marker, together with whatever tells that rows were rewritten. A
database often does not change size when a row is added, and a database in WAL mode may not change
its file time until a checkpoint, so make the stamp from the folder, the `-wal` file or a cheap query,
not from the main file alone. Archive a consistent snapshot, not a copy of the live file: `VACUUM
INTO` or the backup API, then compress it. `tests/support/folder_agent.rs` does this.

## Codex CLI: what a real second agent needed

`src/agents/codex/` reads one rollout per session, and what it had to settle is typical:

- **Which files are sessions.** `files.rs` finds `sessions/YYYY/MM/DD/rollout-*.jsonl` and `archived_sessions/`, one file per thread id (the newest wins), and reads the first line of each once to leave out what Codex's own picker leaves out: `codex exec` runs, helper threads and sub-agents. A sub-agent is a child stream of its parent and its tokens count apart (`subagents`, `read_usage`).
- **Two dialects in one folder.** Older rollouts write prompts and replies as `user_message` and `agent_message` events, newer ones as `item_completed` events. `rollout.rs` decodes both into one `Event`. Prompts come only from those events, never from the model's own copy of the conversation, which holds the injected `AGENTS.md` and environment text.
- **Tool calls are written twice.** A newer (paginated) rollout records each call as the model's raw `response_item` and as an `item_completed` item (`CommandExecution`, `FileChange`, ...) with the command, its output and exit code, or the files changed. Codex runs tools through code mode, so the raw call is a JavaScript program. The items are what eye reads: commands, commits, pull request links (also in the result of an MCP or dynamic tool call, `results.rs`), changed folders and the tool count. Codex has no counterpart of Claude Code's published artifact pages, so there is no such output for it. The raw calls of such a file are skipped (every line of a paginated file has an `ordinal`, a legacy one has none). A legacy file has only the raw calls, and `script.rs` reads the commands and patches out of a code mode program when they are plain string literals; otherwise the summary's tail says `exec`.
- **Totals, not increments.** Every `token_count` line is the thread's total so far. The `Delta` holds the new total minus the one kept in the `carry`, and a total that went down means "read again" (`replaces`). The folder works the same way: the last turn's folder is where a resume belongs, so a change makes the adapter read the file again from the start.
- **A rename is in another file.** `/rename` writes `session_index.jsonl`, not the rollout, so the adapter folds the time of the rename into the stamp and eye reads the session again.
- **Live status without a status file.** An open session holds a lock on `thread-writer-locks/<id>.lock`; `live.rs` tries a shared lock and lets go at once. Busy or idle is the last of `task_started`, `task_complete` and `turn_aborted` at the end of the rollout. A permission prompt is not on disk, only the hook sees it, so the adapter says `live_status_shows_prompts` is false: while a `PermissionRequest` is recorded the session is `waiting` (Needs you, blocked), not `busy`. Nothing says the prompt was approved, so the next `PostToolUse` ends it: late for a long command (the card stays in Needs you until it finishes), early when another tool call finishes first.
- **No session id up front.** `start` returns a plan without one, and the scanner links the idea to the session that begins in its folder afterwards (`ideas::link_sessions`).
- **Hooks.** `hooks.json` has the shape of Claude's `settings.json`, so both adapters use `hooks_file.rs` with their own events and command. Codex runs a hook only after the user trusts it once; eye can say so and cannot see it. `PostToolUse` runs after every tool call and carries its output, so its queue line keeps only the session id.
- **Summaries.** `ask` runs `codex exec` with the flags that keep it from leaving a session behind or running the user's own hooks, checks once with `--help` that this version has them, and hands over the schema in the strict form Codex wants (`additionalProperties: false`, every property required).

## Where a session was started

Each session has an `origin` (schema 17): `terminal`, `headless`, `app`, `desktop`, `ide`, `web`, `night`, or none when unknown, and the `origin_raw` it came from. The board shows a chip for everything but `terminal`, the picker preview says "started from: an app (sdk-ts)", and `eye ls --origin app` filters. `Delta::origin_raw` carries the raw value from the first line that has it, `Agent::origin_raw(path)` reads it from the first lines of a transcript (the backfill for sessions indexed before), and `agents/origin.rs` maps raw values to words in one place. A session whose folder is under `<eye home>/night/` is `night`, whatever it wrote. Summaries run with `--no-session-persistence`, so they leave no session behind to label.

| Agent | What it writes | Maps to |
|---|---|---|
| Claude Code | `entrypoint` on its lines | `cli` terminal; `sdk-cli`, `claude-code-github-action` headless; `sdk-ts`, `sdk-py`, `mcp` app; `claude-desktop`, `claude-desktop-3p`, `local-agent`, `claude-coworker*` desktop; `claude-vscode` ide; `remote*`, `ssh-remote`, `claude-in-slack`, `claude-in-teams` web |
| Codex | `originator` and `source` of `session_meta`, joined as `originator/source` | the source first: `vscode` ide, `exec` headless, `mcp` app, `custom:atlas` and `custom:chatgpt` desktop; else the originator: `codex_cli_rs`, `codex-tui` terminal, `codex_exec` headless, `codex_vscode` ide, `codex_atlas`, `codex_chatgpt_desktop` and `Codex …` desktop |
| agy | `source` of the summaries row (empty in every conversation seen on agy 1.5.14); and its own process log, `Print mode: conversation=<id>`, for a `-p` run, which eye stores as `print-mode` | `print-mode` headless; `CORTEX_TRAJECTORY_SOURCE_CLI` terminal, `_SDK`, `_PYTHON_SDK`, `_AGENT_API` app, `_VSCODE` ide, `_ASYNC_*` web (the enum of agy's binary; none of these was seen on disk); a terminal conversation has none |
| OpenCode | nothing | none |

The Claude Code values are the ones its 2.1.311 binary knows (`CLAUDE_CODE_ENTRYPOINT`); the docs do not list them. The Codex ones are from its `default_client.rs` and from rollouts. A value that is not in the table is stored raw and maps to none.

## Antigravity CLI and OpenCode

What works for each (a dash is "not offered"):

| | Claude Code | Codex CLI | Antigravity CLI (`agy`) | OpenCode |
|---|---|---|---|---|
| Sessions on the board, picker, search, usage | yes | yes | yes | yes |
| Read incrementally | byte offset | byte offset | byte offset of the JSON log | no: a session is read whole when its stamp moves |
| Prompt history of deleted sessions | yes | none | `history.jsonl` | none |
| Conversation, summary tail | yes | yes | yes | yes |
| Resume | `claude --resume` | `codex resume` | `agy --conversation <id>` | `opencode --session <id>` |
| Fork | yes | yes | not possible (agy has no fork flag or command outside its TUI) | no |
| Start an idea | yes | yes (no session id up front) | `agy -i <prompt>` (no id up front) | `opencode --prompt <text>` (no id up front) |
| Tokens and usage by model | yes | yes | yes (model from the database) | yes |
| Live status | status files | writer lock + hooks | presence lock + summaries row + database (busy, idle, waiting) | none |
| Hooks installed | yes | yes | yes: a plugin of eye's (`SessionStart`, `Stop`) | none |
| Summaries (`ask`) | `claude -p` | `codex exec` | `agy -p --output-format json --json-schema` | none |
| Archive | one file | one file | folder: database snapshot + summaries row + the conversation's folder; a restore is resumable | one JSON export; never restored |
| Sub-agents | child streams | child streams | child streams (`parent_conversation_id`) | no |
| `/eye` skill | yes | yes | yes (`~/.gemini/config/skills/eye`) | no |
| Plan limits | yes | yes | not possible: agy keeps none on disk | no |
| Night shift | yes | yes | only with `allow_unsandboxed` (its `--sandbox` is unverified); no fork | no |
| Cost estimate | yes | yes | yes (Gemini prices; GPT-OSS has none) | no |
| Remote Control link | yes | no | yes, from agy's process log | no |
| Forget `--transcript` | yes | yes | yes, also its history lines and cache entries | yes |

**Antigravity CLI.** Built on a real install (agy 1.5.1, rechecked on 1.5.14). A conversation is `conversations/<id>.db`, a SQLite file of
protobuf blobs, with a folder `brain/<id>/` beside it. eye reads the plain JSON log agy keeps there
(`.system_generated/logs/transcript_full.jsonl`: one line per step, with the prompt, reply, tool calls and the
tokens and (from 1.5.14) the model of each model call), the model of older calls from the database (the one protobuf field eye decodes), titles
and folders from `conversation_summaries.db` and prompts from `history.jsonl`. The session is the folder
`brain/<id>/`; its stamp folds in the database, its write-ahead log and the summaries row, so a title that agy
writes later is noticed. agy's databases are in WAL mode and SQLite cannot open one read-only once agy has exited (no `-wal`, no `-shm`); eye then opens it as it is (`immutable=1`). Conversations without a log and the ones eye starts for summaries
(a folder named `eye-ask-<uuid>`; agy has no flag to leave nothing behind) are not sessions.

- **Live status** (`live.rs`). agy holds an exclusive `flock` on `presence/<id>.lock` while a conversation is open (held from the start, released on quit; the files stay behind, so a lock that can be taken means closed). The summaries row says `CASCADE_RUN_STATUS_RUNNING` while a turn runs and `..._IDLE` after: busy and idle. A tool call that waits for approval is the last row of the conversation's `steps` table with status 9 (`CORTEX_STEP_STATUS_WAITING`, 3 is done): `waiting`, "permission prompt". It is in the database only; the JSON log writes a step when it is done. A step waiting for something other than a tool approval is shown as "a question" (not seen, so not verified). Checked in a real session with `rm`, which agy asks to approve.
- **Remote Control** (`--remote-control`, `/remote-control`). The link is not in the database or in presence. agy's process log (`log/cli-<time>.log`, one per `agy` process) has `[remote-control-<uuid>-v2] ... Connection status: Connected`, and the person is shown `https://antigravity.google.com/r/<uuid>-v2`; `Created conversation <id>` in the same log ties it to the conversation. eye sets `remote_url` while that connection is up. It is the process's link, so it covers every conversation that process opened. The same logs have `Print mode: conversation=<id>` for a `-p` run: that is how eye knows it was headless (a live `-p` run reports the entrypoint `sdk-cli`).
- **Sub-agents.** agy's `invoke_subagent` makes a conversation of its own: a row with `parent_conversation_id`, `agent_name` (`self`) and `nesting_depth`, its own `brain/<id>/` log (and no presence lock), and `brain/<parent>/.system_generated/subagents/<id>.json` with the role the parent gave. eye shows them as child streams of the parent (title = role, detail = kind, finished when its turn is not running) and counts their tokens apart.
- **Hooks.** agy reads `hooks.json` from a workspace's `.agents/` folder and from a plugin; there is no global hooks file, but a plugin in the global folder `~/.gemini/config/plugins/<name>/` is loaded in every conversation (checked: `agy plugin list` shows it and its hooks ran in a real session). `eye install` writes the plugin `agent-eye` there (`plugin.json`, `hooks.json` with `SessionStart` and `Stop`); nothing of another tool is touched, and `uninstall` removes only it. agy waits for each hook and a `PreToolUse` hook that prints no decision can block the tool, so eye listens to no tool event and every command ends in `echo '{}'`, an empty decision, whatever eye did. `Stop` hooks were not seen in a `-p` run; the file-based status does not need them. The hook payload is camelCase JSON with `conversationId`, `cwd` and `transcriptPath`; the queue line keeps the session, the folder and the end of the last reply (to tell a question from a finished job).
- **The `/eye` skill** is `~/.gemini/config/skills/eye/SKILL.md` (`skill/SKILL.agy.md`, the Codex text with Antigravity's name; it says agy cannot fork). The daemon refreshes it after an upgrade like the others.
- **Cost.** The log names the model of each call (`gemini-3.8-flash-low`) and the database a display name (`Gemini 3.8 Flash (High)`); both are priced (see `usage.md`).
- **The archive copy** is a folder (`eye-conversation.db.zst`, a `VACUUM INTO` snapshot, `eye-summary.json.zst`, the conversation's row, and every file of `brain/<id>/` compressed under `files/`, without `scratch/` and files over 8 MiB). Restoring into agy's own folder puts the snapshot at `conversations/<id>.db` and the row back, so `agy --conversation <id>` opens the conversation after agy deleted it (checked on a real conversation: deleted, restored, resumed with `agy -p --conversation`). A copy read for the board keeps its snapshot beside it.
- **Forgetting** (`eye forget --transcript`) removes the folder, the database and its `-wal`/`-shm`, the presence lock, `annotations/<id>.pbtxt`, the row (zeroed: `secure_delete`), the conversation's lines in `history.jsonl` (so nothing is left in agy's up-arrow history) and its entries in `cache/last_conversations.json` and `cache/conversation_metadata.json`. agy's process logs under `log/` name the id and are left alone.
- **Not possible.** Fork: no flag or command outside the TUI. Plan limits: agy keeps none on disk. Origin of a terminal conversation: `source` is empty in every row seen. A `-p` night run is not confined by anything eye could verify (see `night.md`). Not read: commits and folders as outputs, background tasks.

**OpenCode** (checked on a real OpenCode 2.3.2: a session is listed with its title, folder, conversation and
tokens, a second message is picked up at the next scan, and `--session` / `--prompt` resume and start it). The
adapter follows the structs of sst/opencode v2.3.2 (and v1.1.1 for the older files) and is tested in all three
layouts: the `session_v2` tables, the older `session`/`message`/`part` tables (both in `opencode.db`, found in
`$XDG_DATA_HOME/opencode` or `~/.local/share/opencode`) and the JSON files of v1.0 and v1.1
(`storage/session|message|part`). A session of a database is no file, so its transcript path is `<db>#<id>` and
`transcript_exists` asks the database. OpenCode's extensions are JavaScript plugins, so there are no hooks, and
`opencode run` cannot answer with a schema, so it cannot write summaries (set `[summary] agent` to another agent).
Sub-agent sessions (with a parent) are left out. The older table and JSON layouts are tested on synthetic
data only.

## Adding an agent

1. Make `src/agents/<id>/mod.rs` with a type that implements `Agent`. Start with the required
   methods and a `build(&AgentCfg) -> AgentRef` function that reads `[agents.<id>] dir`.
2. Add the id to `factory()` and `registered_ids()` in `src/agents/mod.rs`, next to `claude`, `codex`, `agy` and `opencode`.
3. Turn it on in `config.toml`: `[agents] enabled = ["claude", "<id>"]` and, if its files are not in
   the usual place, `[agents.<id>] dir = "..."`. `eye install` does the first part for an agent
   whose `detect` finds it.
4. Add the optional parts one at a time: live state, hooks (`hook <Event> --agent <id>`; the form
   without `--agent` means Claude Code and must keep working), sub-agents, history, summaries,
   install and doctor.
5. Test it the way `tests/agents.rs` does. `tests/support/fake_agent.rs` is a complete adapter in
   about 150 lines with its own file format: it is registered with `agents::register` and then runs
   through the scan, the board API, search, resume commands, conversations, the archive and ideas
   next to Claude Code. `tests/support/folder_agent.rs` is a second one that keeps each session as a
   folder with a SQLite database; `tests/agent_storage.rs` runs all three together and checks the
   marker, a session that is rewritten, and the archive of a folder.

Things an adapter must not do: run a shell with session data in it (return arguments, eye quotes
them), write outside the agent's own folders, or read credential files.

## The archive

The archive keeps a copy of each session per agent. Claude Code's are under `archive/<project dir>/`,
every other agent's under `archive/@<id>/<project dir>/`, named by `archive_name` (`<id>.jsonl.zst`
by default). eye decides where and when; the adapter decides how:

- `archive(path, copy)` writes a copy of the session at `path` to `copy`, a path that does not exist
  yet. It may make a file or a folder. Make files readable by their owner only.
- `unarchive(copy, dest)` makes the session again at `dest`, a path that does not exist yet. The
  result must read the same as the original.

eye uses `unarchive` in two places. Restoring a deleted session before it is resumed (the original
path, written through a temporary name and then renamed). And reading the conversation of a session
the agent has deleted: eye restores a copy into a temporary folder in `~/.agent-eye/tmp`, hands that
path to `read_conversation`, and removes it afterwards. So the same code reads a live session and an
archived one.
