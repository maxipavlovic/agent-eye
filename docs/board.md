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

Search (every session, archive included; the best match first), the Needs you count, the live connection, **Tonight: N** while night runs are planned (see `night.md`), USAGE (cost and plan limits, see `usage.md`), the **settings gear** and **+ IDEA**. The theme, look, accent, layout, folded lanes and the panel sections you keep open are shared by every browser on this machine (`ui.json`); each look keeps its own layout, folded lanes and open sections.

## Settings

The gear (or the `,` key) opens SETTINGS. Every change applies at once and the panel says **Saved**.

- **Look**: INK or IRIS, the eight accent colours, Lanes or Grid, and light, dark or auto.
- **Board**: which lanes are folded.
- **Summaries**: on or off, and the model (`[summary] enabled`, `model`).
- **Night shift**: on or off, the window, how many runs at once and where runs work (`[night] enabled`, `window`, `parallel`, `isolation`). The window is checked as `eye night window` checks it.
- **Reminders**: the desktop notice (`[remind] notify`).
- **Agents**: which agents eye follows (`[agents] enabled`), in the order of the file; new ones are added at the end, and the first is the default.

They are written into `config.toml` (comments and other keys stay), through `/api/settings` (see `api.md`). The background service reads the night shift and reminder keys again on each pass; summaries and agents count after it restarts, and the panel says so. Folders, prices, exclude lists and the night sandbox switches are not in the panel: they stay in `config.toml` (see `config.md`). On the sample board (the site's demo) the settings are shown, but nothing is saved.

**Looks.** Ink is pixel art on warm paper. Iris draws the board as a technical drawing in one-pixel lines: header cells, big condensed counts, and under each lane name one fact (how long the oldest ask has waited, who you are waiting on, how many are working) and a timeline with one tick per session, two weeks ago on the left, now on the right. A title block under the board counts what is on it. Its drawings move a little: the eye blinks, the bell rings while something waits on you, the hourglass turns over while you wait on others; with reduced motion set in your system, nothing moves on its own. Both have the same cards, panel, keys and colours. Switching looks loads the page again; `?look=iris` in the address shows one for that page only and saves nothing.
