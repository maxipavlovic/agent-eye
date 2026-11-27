# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- Turn timing is now recorded locally: when you prompt, when the agent finishes, interrupts, tool calls and errors, refusals, thinking and compactions (timings and sizes only, never text). Sessions already indexed are read again once, in the background. New `[clock]` settings in `config.toml`.
- Turn timing covers Codex CLI, Antigravity CLI and OpenCode too, with every kind their files allow; `docs/agents.md` lists which and why not.
- Turn timing follows the same rules as the index: sessions in excluded or temp folders get none, `[clock] enabled = false` stops new turns, a turn has one `end` at the agent's last activity in it, and a prompt or end a forked session copied from its parent counts once.
- A **settings gear** in the header of both looks (or the `,` key). It holds the look, colour, layout and theme that used to crowd the header, which makes room for USAGE and what comes next to it, and the settings that so far lived only in `config.toml`: summaries (on, model), the night shift (on, window, at once, where runs work), desktop notices for reminders, and which agents eye follows. Each says in one line what it does and saves at once.
- `GET /api/settings` and `POST /api/settings`: a whitelist of config keys with their types and allowed values, written under the same lock as the CLI, comments kept; an unknown key or a bad value is a 400 and nothing is written (see `docs/api.md`).
- `eye clock`: the steering clock. It shows your correction rate by hour of the day and by time since your last break, which hours run clearly worse than the rest (only when the difference is more than chance), and whether a bad stretch looks like you or the model, from the model's thinking and its failed tool calls in the same hours. `--weeks N`, `--json`, `--brief` (one line, for an agent). A model judges which prompts were corrections once a night (and with `eye clock detect`), through the agent and model that write summaries; `[clock] llm = false` stops it. The board's API gets `GET /api/clock`. See `docs/clock.md`.
- `[clock] idle = true` (off by default) takes breaks from the machine's idle time on macOS and on Linux with logind, instead of from gaps between prompts. It reads one number once a minute, never keys or windows. `eye doctor` reports whether it is available.
- `GET /api/usage` takes `series=project|model` and answers with the cost of every day split that way, and `eye usage --by-day` prints one row per day (see `docs/api.md`).
- A **HOURS** button beside USAGE opens the steering clock: your corrections per 100 prompts for each hour of the day, how much the model thought against its usual, and failed tool calls, on one 24-hour axis with your bad hours shaded, plus corrections by time since your last break. Every hour has a tooltip, there is a table view, and thin data reads plainly ("Not enough data yet"). With `[clock] hints` on, a quiet mark beside HOURS and on the card you are working in says when your corrections usually rise.
- USAGE draws the cost of every day as stacked columns, by project or by model, with a legend, tooltips and a table view.
- The settings gear has a HOURS section: the clock, judging corrections, the hint, the break length and idle time (`clock.*` in `/api/settings`).
- With `[clock] hints = true`, the picker's footer shows the clock's hint after the keys when the screen is wide enough. It is worked out in the background, so the picker still opens at once.
- While the hint is set, **Continue tonight** comes first in the right-click menu, in MORE and in the panel's actions of the live session you are working in, with the reason in one plain line. T does what it did.
- The steering clock's verdict is tested by chance properly. The bad hours are picked because they look worst, so they are now tested by running the whole pick-and-measure procedure again on corrections dealt out at random (seeded, 1,000 rounds, a few milliseconds), and the model's thinking the same way; on made-up weeks with no pattern the old test named hours in more than half. See `docs/clock.md`.
- The clock leaves out sessions started by a script (`claude -p`, `codex exec`) or by the night shift, since those prompts are not you steering. The nightly pass also waits while an agent is working, not only after your own prompts.
- The clock's hint comes from the last 4 weeks wherever it is shown (board, picker, `eye clock --brief`). The tool-error strip of HOURS shows one agent's calls (`tool_agent` in `/api/clock`), as the verdict compares each agent with itself. "Too few to tell yet" with judging off names the real reason.
- The corrections pass skips OpenCode as the summary agent: its locked-down run has not been checked on a real install.
- The steering clock's verdict gates the model's side as strictly as yours: failed tool calls are tested per agent against that agent's own rate (never pooled across agents), thinking needs 30 thoughts inside and outside the hours, and a signal with too little data says "no model-side data" instead of "flat". The hours the sentence names are always the hours of the chart, each named by one rule (morning, afternoon, evening, night). `/api/clock` is additive: `judging` and `judging_reason` (why corrections are zero), `think_agent`, `bad[].label`, per-agent `think_unit`, `think_ratio`, `tool_error_ratio` and `signals`, the verdict kind `unmeasured`, and `now.hint` as `{text, offer}`. `eye clock` prints a line per agent.
- A break is an idle stretch or a gap between prompts, so a night with no idle sample is still a break. On Linux the idle sensor finds your graphical session through `loginctl`, and `eye doctor` reports what the daemon itself can read.
- The clock reads its settings again: changes made in the gear count without a restart (recording turns follows the daemon's start). The nightly corrections pass waits while you are at work.
- The call that judges corrections is locked down for OpenCode (the shell, edits and web fetches are denied) and cleans up after agy (its conversation is deleted). `docs/clock.md` and `docs/security.md` say what each agent's call can and cannot do.
- HOURS draws its single series in the look's ink, not a colour: colour stays for "needs you". Its hint mark is ink too. The model strip is cut at 50% either side of usual with a mark where a swing is cut, shows a tick for an hour that was about usual and fades an hour with few thinking events. The bad stretches are named by the engine ("your night" for 22:00 to 04:00) and labelled where they sit.
- USAGE's stacked colours are a new palette, checked for both looks and both themes, that leaves out the colours near your accent colour, so no column looks like "needs you". On a phone (under 640 px) the HOURS and USAGE drawings are laid out again at 320 wide, and at a desktop width their words are 15 px in the drawing, so they stay readable. USAGE's columns are one control read with the arrow keys, like HOURS, and a tap on either chart reads the hour or day you touched.
- HOURS says why corrections are zero (the engine's reason) when they are not judged, lists each agent with its own thinking and tool errors ("—" where it has no such signal), names the thinking unit of the agent the hours belong to, and keeps the chart on screen while another number of weeks loads.
- The settings rows are named after what they set ("Write summaries", "Start by themselves", "Follow", "Skin") and the form labels match them.
- A settings write that fails on disk is a 500, not a 400, and HOURS says what failed unless the engine simply has no clock.
- The Steering clock switch in the gear says it takes effect when the service restarts, as the docs do.
- `eye usage --by-day` follows `--by project|model` and refuses the other groupings; `eye clock --help` reads as two sentences and describes `--json`; an empty break band says "no prompts".

### Changed

- The header no longer carries the Lanes / Grid, theme, look and colour buttons: they are in the settings panel, and their keys and saved settings are unchanged.

## 0.4.1 - 2026-11-25

### Fixed

- An Esc pressed the moment a menu or list opened (MORE, MOVE TO…, a card's right-click menu, USAGE, the Tonight list) could reach the board instead and close the card's panel under it. Each now listens for Esc from the moment it shows.
- Linux: copying to the clipboard and desktop notices no longer fail now and then with "text file busy", as opening a terminal already did not.

## 0.4.0 - 2026-11-25

### Added

- A second look for the board, **iris**: the board as a technical drawing in one-pixel lines, with header cells, condensed counts, a fact and a timeline under each lane name, and a title block. Its drawings move a little (the eye blinks, the bell rings while something waits on you) and keep still when your system asks for reduced motion. Pick INK or IRIS in the colour button of the header (the choice is shared by every browser, like the theme). Same cards, panel, keys and accent colours as ink.

### Changed

- The board's settings keep one layout, set of folded lanes and open panel sections per look (`eye.<look>.layout`, `.fold`, `.panel`); ink keeps the ones it had. `ui.json` holds at most 64 settings.
- Dependencies: jiff 1.0 (its `Zoned` type is now `ZonedDateTime`; nothing changes for you) and clap 4.6.8. CI: taiki-e/install-action 2.90.5 and sonarqube-scan-action 8.5.0.
- A card without a reminder says "No" under REMINDER in its panel, not "none".
- `?look=<name>` in the address shows that look for that page only and saves nothing. Before, the browser kept it until the engine's saved look replaced it.

### Fixed

- A card's right-click menu opened at the top of the window for most cards: too long to fit under the pointer, it flipped above it and was pushed to the top edge. It now opens at the pointer and moves up only as far as it must.
- From the keyboard: in a long card menu, the arrow keys now scroll to the item they reach; Tab out of the menu goes on from the card, not from the end of the page; the key list (`?`) scrolls with the keys in Safari.
- The idea form, the reminder form, the Forget confirm and the key list scrolled as a whole, so on a short window SAVE IDEA was below the edge. Their title and buttons now stay in place and only the fields between them scroll.

## 0.3.2 - 2026-11-23

### Added

- An idea's panel has keys: S starts it, B starts it in the background, T plans it for tonight, E edits it and M sets a reminder. `?` lists them while an idea is open.

### Fixed

- An idea's panel had its buttons on three rows (START, IN BACKGROUND and EDIT; then TONIGHT; then ARCHIVE). They are one row now (START, IN BACKGROUND, TONIGHT, EDIT), with REMIND ME and ARCHIVE in MORE ▾, as a session's panel has them.
- A card whose agent is stopped until you answer was hard to notice: only its border pulsed. Its `!` on the top edge now blinks too, filled and hollow.
- A doc Claude Code writes through the Claude Docs connector (its `batch` tool) is an artifact output, with the doc's name as its title. Before, only the `Artifact` tool's publishes counted, so a session that made a doc showed no artifact. Sessions indexed before pick it up with `eye scan --full`.

## 0.3.1 - 2026-11-22

### Changed

- Dependencies: rusqlite 0.43 (SQLite from libsqlite3-sys 0.40), sha2 0.11, vite 8.3.8 (face).
- Site: a shorter headline ("Keeps an eye on your agent sessions, even overnight") and pitch, shorter feature cards, code in the docs wraps instead of scrolling sideways, and the demo board leaves its theme switch to the page's so + IDEA fits. The README uses the same headline.
- Site: the docs have search. A box at the top of the docs menu (`/` focuses it) looks through every section of every page: every word must match, headings and titles count most, and the arrow keys and Enter open a result.

### Fixed

- On a narrow screen the board scrolls only its lanes to Needs you; it used `scrollIntoView`, which also scrolled a page it is embedded in (the site jumped down to its demo on load).

## 0.3.0 - 2026-11-22

### Added

- Antigravity CLI (`agy`) support on a par with Codex: live status (busy, idle, waiting for your approval) from its presence lock, summaries row and database; its Remote Control link and `-p` runs from its process log; sub-agent conversations as child streams; a global plugin with `SessionStart` and `Stop` hooks and the `/eye` skill in `~/.gemini/config`; Gemini prices for the cost estimate; `eye forget --transcript` also clears its history lines and caches; a restored archive is a conversation `agy --conversation` can open; night runs only with `[night] allow_unsandboxed = true` (new `[night] agy_model`). No fork, plan limits or terminal origin: agy offers none. See `docs/agents.md`.
- `eye forget <id>... | --folder <path> [--transcript] [--yes] [--dry-run]` removes sessions from eye (the session, its search text, its other rows and its archive copy) and, with `--transcript`, the agent's own copy: Claude Code's transcript and sub-agent folder, Codex's rollout and its `session_index.jsonl` line, agy's conversation folder, database and summary row, OpenCode's rows or JSON files. It asks first, refuses a session that is open now and takes a `manual` backup snapshot before removing anything. The board has FORGET… in the card menu and the panel (key X) with a confirm and a box for the agent's copy (`POST /api/sessions/:id/forget`).
- Where a session was started: T3 Code, Conductor, the desktop app, an editor, the web or a headless run. Its name (App, Headless, Desktop, IDE, Web, Night) in the card's top line and the panel's facts line for anything but the terminal, "started from: an app (sdk-ts)" in the picker preview, `origin` and `origin_raw` in `eye ls --json` and the card JSON, and `eye ls --origin app`. Sessions indexed before are filled from the first lines of their transcripts, with no re-read. `docs/agents.md` has the table.
- Leftovers: each session lists what it left behind. Servers and shells still running (on a port, or in the session's folder or scratchpad), containers, git worktrees in a scratchpad or whose folder is gone, commits on no remote and modified files nobody committed. A card shows the count in a box on its top edge; the panel has a two-line LEFTOVERS block (what is left, by kind, with REMOVE ALL… for what eye can stop or remove and IGNORE ALL; SHOW opens one line per item with its own actions); the picker's preview has a line and Ctrl-L opens the list; `eye loose` lists them and `eye loose stop|remove|ignore <key>` and `eye loose send <session>` act. Stop sends SIGTERM to a process whose pid and start time still match; remove runs `git worktree remove` without `--force`; commits and files only get a command to copy. Every action asks first. `[loose] enabled` and `days` in `config.toml`.
- Antigravity CLI (`agy`) and OpenCode as agents (`[agents] enabled = ["agy", "opencode"]`; `eye install` turns on the ones it finds). agy: read from its JSON log, with titles, tokens by model, history, archive and summaries (see the agy entry above for live status, hooks and the rest). OpenCode: checked on OpenCode 2.3.2, with its older database and file layouts read too (see the OpenCode entries below for live status, summaries, restore and night runs). New `Agent::transcript_exists` for sessions that live inside a database.
- Night shift (off until `[night] enabled = true`): plan an idea ("TONIGHT") or a session ("CONTINUE TONIGHT") and the daemon starts it, headless, inside a window (01:00-07:00 by default) when no session you work in is live. Each run works in its own git worktree on a new branch (or a copy of a plain folder), and runs in parallel with others up to `[night] parallel`. A Claude run's Bash can reach only package registries (`[night] network`; no git host) and `git push`, `gh`, `ssh` and the publish commands are denied; Codex runs have no network. Claude's own tools (WebFetch, MCP) are not limited by that list. Each run can set one limit (weekly plan above N%, a token budget, or minutes); a limit eye cannot check is refused when planned. A run whose plan runs out can start again on the other agent (`--switch`). In the morning the run's session is a Needs-you card with a night mark, its report, diff stat and leftovers, and RESUME, OPEN FOLDER and DISCARD. `eye night on|off|add|ls|cancel|log|window|parallel|run-now|discard`, a Tonight chip and list on the board, `/api/night`, and `docs/night.md`. A `codex exec` rollout is a session only when it ran in a night run's folder.
- Night shift has two ways to keep your folder safe: `worktree` (the default, as before) and `copy`, a separate local clone with no `origin` where your folder is locked completely, `.git` included. `[night] isolation`, `eye night add --isolation worktree|copy`, and a **Work in** choice in the TONIGHT and CONTINUE TONIGHT forms; the NIGHT RUN section shows which one a run used. For a repo copy, **BRING INTO MY REPO** (`eye night bring <run>`, `POST /api/night/:id/bring`) fetches the night branch into your repo after you confirm, and refuses when the branch exists. `night_run.isolation` is a column of schema 17. `docs/night.md` explains when to pick which.
- `eye usage [--days N | --week | --month] [--by project|folder|agent|model|session] [--json]`: tokens and an API-equivalent cost estimate (not your bill) per day, project, agent, model or session, with plan limits on top. Prices live in one file with their sources; `[usage.prices]` adds or overrides a model, and a model without a price shows tokens and no cost. Sessions from before are counted in the background.
- A USAGE panel on the board (this week by project and by agent, the top 10 sessions, four weeks, the limit lines) and `GET /api/usage`; tokens and cost on the session panel, in `eye show` and in the picker preview.
- Codex plan limits, kept as the newest seen. For Claude Code, an opt-in `eye statusline [--wrap '<cmd>']` status line command stores the limits Claude Code hands to its status line; `eye install` does not turn it on (see `docs/usage.md`).
- `eye export` writes one `.eyepack` file with everything eye knows (database, transcript archive, and copies of transcripts the agent still has), and `eye import <file>` reads it on this machine or another one: a fresh install takes the pack's database, an existing one gets the missing sessions and ideas merged in (the newer row wins). Home folders are rewritten for the new machine (`/Users/max` to `/home/max`, also in Claude's `-Users-max-…` folder names). Import shows the plan first, asks `[y/N]` (`--yes`, `--dry-run`), and keeps a `pre-import` snapshot. The board's token is never in a pack. `eye export --to -` and `eye import -` stream a pack through standard output and input, so `age -p` or `gpg -c` can encrypt it on the way (eye has no encryption of its own).
- `eye export <session> [--html|--md]` writes one session as a readable, self-contained page (secrets masked, tool calls folded to a line), from the archive when the agent deleted the transcript. The board's session panel has an EXPORT button (`GET /api/sessions/:id/export`).
- The terminal preview says when a session's folder does not exist on this machine.
- Database schema 14 (`loose` and `loose_ignore`).
- Database schema 15 (`usage_day`, `usage_state`).
- Database schema 16 (`night_run`).
- Database schema 17 (`session.origin`, `session.origin_raw`).
- A card has one fixed frame: a top line in plain words (the project, the agent when there is more than one, where it was started) that never wraps, the age, a two-line title and the next step. Its states are small boxes on the top edge, most important at the right: Needs you, a reminder, a night run, leftovers (their count) and Remote Control, at most four (`+N` for more). Tokens are no longer on the card (the panel, USAGE and the picker have them; the API field stays). A test screenshots a card that carries everything at three widths.
- The session panel: what to do at the top (the moves and RESUME, a MORE menu in the top bar with every other action and its key, the state, the next step, the night run in one line, LEFTOVERS, and the forms you open, such as continue tonight); then what happened (recap, topics, sub-agents, outputs, the first prompt, where). Only the long or seldom needed parts fold to one line until opened: the night run's details, the conversation, tokens and the reminder. eye keeps the ones you open open (`eye.ink.panel` in `ui.json`), and a search that matches inside one opens it. The panel still ends with the resume command and the action row.
- `/eye` on its own gives a short overview: what needs you, what runs now, what sessions left behind and what is planned tonight. The skill also knows `eye usage`, `eye loose`, `eye night` and `eye forget`. After upgrading, run `eye install` once to bring the installed `/eye` skills (and the OpenCode plugin and command) up to date.
- Board panel keys: the existing N, W, P, D and R stay as they were; new are M (remind me), T (continue tonight), F (fork), C (copy the resume command), H (hide), L (leftovers), E (export) and X (forget). `?` lists them; the moves and RESUME show their letter, and the panel's MORE menu shows the key of each other action.

- OpenCode on a par with Codex, checked on a real OpenCode 2.3.2 with free models: live status (running and idle from the turn times on its session row, waiting on a permission from a small plugin that `eye install` puts in `~/.config/opencode/plugins/` and `uninstall` and `doctor` know), summaries (`opencode run --format json`, validated against the schema, one retry, `--archived` and deleted afterwards; default model `free`), the `/eye` command (`~/.config/opencode/commands/eye.md`), archive and restore through `opencode session export` and `import`, `$0` cost for `-free` models and the Claude and OpenAI prices for `provider/model` names, and OpenCode as a night agent (`[night] opencode_model`; refused unless `allow_unsandboxed = true`, since it has no sandbox). Not possible, and said in `docs/agents.md`: prompt history of deleted sessions, plan limits, origin and a fork from its terminal UI.
- New `Agent::restore_archived` for an agent that puts a session back through its own import, and the daemon applies a wait that arrived before the scan indexed a new session.

### Changed

- Search is one engine for the picker, the board and `eye find`. Word forms match (`tokens` finds `token`), a mistyped word is fixed from your own sessions, and half of the words are enough. A pasted link finds the session that made it. Titles and rare words count most; status and age no longer lift a session, so old sessions are not pushed to the bottom. Archived sessions still come last.
- `eye find --json` and `/api/search` add `relevance` (higher is better). `score` keeps its 0.2 order, lower is better: it is `relevance` negated.
- Database schema 13 (the search index stems words; 0.2.x still reads it).
- Dependencies: axum 0.8, zstd 0.14, toml 1.2, jiff in place of chrono, vite 8.3.7 (face).
- `config.toml`: a section or key this version does not know is listed once on standard error and ignored; text that is not TOML is still refused.
- Sessions started in a temp folder (`/tmp`, the system temp folder, Claude's scratchpads) are not indexed, so test and scratch runs stay off the board. `[exclude] temp = false` indexes them again (for testing). Sessions indexed before stay until you forget them (`eye forget --folder /tmp`).
- Night runs: a run is claimed before it starts, so two starters or a cancel cannot start it twice; a run whose process start time cannot be read is marked lost and never signalled. In `acceptEdits` mode only build and test commands are allowed.
- `eye import` takes the `pre-import` snapshot before copying files, skips files that already match, and refuses a pack that unpacks to more than its manifest says. `POST /api/loose/stop|remove` need `confirm: true`. The usage backfill runs in its own thread.
- Going back to 0.2.x works: it ignores the new config sections and still reads and writes the schema 17 database.

### Fixed

- The accent colour picker now closes on an Esc or a click outside that comes right after it opens.
- The reminder's date picker moved from the old day when two keys came before it was drawn again (Down then PageDown from late in a month landed a month short).

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
