# The board

The board is a web page the daemon serves on your machine: every session as a card, in lanes that say who acts next. `eye board on` turns it on and `eye board` prints its address (`http://127.0.0.1:7420/?t=<token>`); `eye board off` turns it off. It is off by default, and only your browser on this machine can open it (see `security.md`).

## Lanes

**Ideas** (parked work, with START and TONIGHT), **Running**, **Needs you** (it waits on your answer or your review), **Waiting** (on someone else), **Paused** and **Done** (folded to a rail; click it to unfold). The Lanes / Grid switch in the header lays the same lanes out as columns or as wide sections. The bar under the header filters by project: the five biggest, then `other`, each with how many cards need you.

## A card

Every card has the same frame, whatever it carries:

- **The top line**: the project, the agent (when more than one is turned on) and where the session was started when it was not a terminal (App, Headless, Desktop, IDE, Web), in plain words on one line that ends in "…" when narrow; the age at the right.
- **The title** (two lines at most), then **the next step** with who acts: `YOU`, the agent, or a person's name. A running card shows what it is doing instead. A reminder adds its time.
- **Marks on the top edge**, the most important at the right: `!` (needs you), a clock (a reminder is set), a crescent (a night run made it), a number (its leftovers) and the Remote Control signal. At most four show; more become `+N`. Hover a mark for its words.

A card whose agent is stopped until you answer pulses its border (ink) or has its top rule in the accent (iris), and its `!` blinks. Right-click a card (or press the menu key on it) for its menu: MARK DONE, MOVE TO, REMIND ME (tomorrow, Monday, in a week, a date), HIDE, FORGET…, COPY RESUME COMMAND, FORK, CONTINUE TONIGHT… and OPEN DETAILS.

## The panel

Click a card to open its panel on the right; ← and → walk to the next card.

- **At the top**: the moves (NEEDS YOU, WAITING, PAUSED, DONE, and AUTO to give the card back to the rules) and RESUME. MORE ▾, beside `?` and ESC, has the other actions with their keys. Under them: the state, the next step, the reminder, the night run in one line (REVIEW opens its details) and LEFTOVERS (what the session left behind, by kind, with REMOVE ALL… and IGNORE ALL; SHOW lists each item with its own actions). A form you open, such as CONTINUE TONIGHT, appears here.
- **Then what happened**: recap, idea notes, topics, sub-agents, outputs (pull requests, artifacts, Jira keys, commits, folders), the first prompt, and WHERE (folder, branch, start, the days it was worked on, cost).
- **Folded to one line until you open them**: the night run's details, the conversation, tokens and the reminder's choices. eye keeps the ones you open open, and a search that matches inside one opens it.
- **At the end**: the resume command and the action row.

Every action that changes something outside eye (stop a process, remove a worktree, forget a session, discard a night run) asks first, in the page.

## Keys

In a session's panel, when no field has the focus:

| Key | Does |
|---|---|
| R | Resume |
| N, W, P, D | Move to Needs you, Waiting, Paused, Done |
| M | Remind me (opens the picker) |
| T | Continue tonight |
| F | Fork |
| C | Copy the resume command |
| H | Hide |
| L | Leftovers (opens the list) |
| E | Export the session as one page |
| X | Forget |
| ← → | Previous or next card |
| / or ⌘K | Search |
| , | Settings (the gear) |
| Esc | Close |
| ? | The list of keys |

An idea's panel has START, IN BACKGROUND, TONIGHT and EDIT in one row, and REMIND ME and ARCHIVE in MORE ▾. Its keys:

| Key | Does |
|---|---|
| S | Start |
| B | Start in the background |
| T | Plan for tonight |
| E | Edit |
| M | Remind me |
| ← → | Previous or next card |
| / or ⌘K | Search |
| , | Settings (the gear) |
| Esc | Close |
| ? | The list of keys |

## The header

Search (every session, archive included; the best match first), the Needs you count, the live connection, **Tonight: N** while night runs are planned (see `night.md`), USAGE (cost and plan limits, see `usage.md`), HOURS (the steering clock, see `clock.md`), the **settings gear** and **+ IDEA**. With `[clock] hints` on, a quiet mark beside HOURS says when your corrections usually rise; it opens HOURS, and it shrinks (to its dot, in the end) before the gear and + IDEA lose their place. It is worked out over the same four weeks as HOURS and the picker. The theme, look, accent, layout, folded lanes and the panel sections you keep open are shared by every browser on this machine (`ui.json`); each look keeps its own layout, folded lanes and open sections.

## Hours

HOURS opens the steering clock (`clock.md`) with the verdict in one sentence first, then three strips on one 24-hour axis in local time, your bad hours shaded and named ("your evening", the engine's own name for them):

- **You**: corrections per 100 prompts, hour by hour. An hour with fewer than 20 prompts is faint with a dotted cap.
- **The model**: its thinking against its own usual (100% is the median hour). A tick at 100% marks an hour that was about usual, so it never looks empty; a swing past half is cut at the edge and marked.
- **Tool errors**: failed calls per 100.

Every hour has a tooltip (hover, a tap, or the arrow keys: the chart is one control, like a slider), and TABLE gives the same numbers as tables. A second chart shows corrections by time since your last break, and BY AGENT lists each agent's prompts, corrections, thinking and tool errors, with "—" where an agent does not report a signal. 2, 4 or 8 weeks. When corrections are not judged (`[clock] llm` off, or no agent can), the panel says why and corrections stay at zero. The charts are drawn in the look's ink, because one series needs no colour; colour is kept for the signal. On a phone the drawings are narrower so their words keep their size.

## The usage chart

USAGE draws the cost of every day as stacked columns, by project or by model (the top five keep their names, the rest are "other"), with a legend, a tooltip for each day and a TABLE view. A name keeps its colour while the panel is open. The columns are one control read with the arrow keys (or a hover or a tap), and on a phone the chart is laid out again at 320 wide with fewer day labels, as HOURS is. Every pair of the seven colours is told apart from every other, and they skip whichever ones sit near your accent colour, because the accent means "needs you"; change the accent and the chart is drawn again without them. A green accent leaves too few, so then the fifth colour may be one that is only a little nearer to the accent (never under 13, where 15 is "near"). Every pair is at least 15 apart for full colour vision and at least 6 apart under protan or deutan simulation, the floor for charts that also have a legend, a tooltip and a table.

## Settings

The gear (or the `,` key) opens SETTINGS. Its sections fold: only LOOK is open when the panel opens, and a folded heading says what is inside (`SUMMARIES  ON · haiku`, `NIGHT SHIFT  OFF`, `AGENTS  Claude Code, Codex CLI`), so the panel reads at a glance. A heading opens or folds its section. Every change applies at once and the panel says **Saved**.

- **Look**: the skin (INK or IRIS), the eight accent colours, Lanes or Grid, and light, dark or auto.
- **Summaries**: on or off, and the model (`[summary] enabled`, `model`). The model is a choice: the summary agent's default first (`Default: haiku` for Claude Code), then the other names that agent takes (`sonnet`, `opus`), and **Other…**, which opens a field for any other name.
- **Started by programs**: a switch for each kind of program that started sessions on this machine, App (an app on an agent's SDK) and Headless (`claude -p`, `codex exec`), each with how many sessions it has and how many quiet takes out of Needs you (`[board] quiet`, off by default). Its folded heading says what it found (`5 FOUND · OFF`, `QUIET`). A quiet session never moves to Needs you by itself, gets no summary and stays out of HOURS; it is still listed, found and resumable, and a reminder or a move by hand still puts it in Needs you. The board follows the switch at once. With none found, the section says so and has no switch; an origin that is quiet keeps its switch when none of its sessions are left.
- **Night shift**: on or off, the window, how many runs at once and where runs work (`[night] enabled`, `window`, `parallel`, `isolation`). The window is checked as `eye night window` checks it.
- **Reminders**: the desktop notice (`[remind] notify`).
- **Hours**: judging corrections, the hint beside HOURS, how long a gap counts as a break, and idle time (`[clock] llm`, `hints`, `break_minutes`, `idle`; see `clock.md`). They count at once, and are off while the clock is. The steering clock itself is shown as ON or OFF, not changed here: it needs the background service to restart, so the row says to set `[clock] enabled` in `config.toml` and run `eye restart`.
- **Agents**: the agents eye follows, in the order of the file (the first is the default), and, quieter, the ones it knows but does not follow. They are shown, not changed here, for the same reason: set `[agents] enabled` in `config.toml`, then run `eye restart`.

Lanes fold on the board itself: a click on a lane's head folds it, and a click on the folded lane opens it again.

The settings are written into `config.toml` (comments and other keys stay), through `/api/settings` (see `api.md`). The background service reads the night shift, reminder, clock and quiet keys again on each pass; summaries count after it restarts, and the panel says so. Folders, prices, exclude lists and the night sandbox switches are not in the panel: they stay in `config.toml` (see `config.md`). On the sample board (the site's demo) the settings are shown, but nothing is saved.

**Looks.** Ink is pixel art on warm paper. Iris draws the board as a technical drawing in one-pixel lines: header cells, big condensed counts, and under each lane name one fact (how long the oldest ask has waited, who you are waiting on, how many are working) and a timeline with one tick per session, two weeks ago on the left, now on the right. A title block under the board counts what is on it. Its drawings move a little: the eye blinks, the bell rings while something waits on you, the hourglass turns over while you wait on others; with reduced motion set in your system, nothing moves on its own. Both have the same cards, panel, keys and colours. Switching looks loads the page again; `?look=iris` in the address shows one for that page only and saves nothing.
