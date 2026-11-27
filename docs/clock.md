# The steering clock

Do you steer an agent worse at some hours, and when a day goes badly, was it you or the model? The
clock answers from your own history: when you prompted, when the agent finished, how often you had
to correct it, and how the model itself behaved in the same hours.

```
eye clock                # the report, last 4 weeks
eye clock --weeks 8      # 1 to 12 weeks
eye clock --brief        # one line: the verdict, and whether it is such an hour now
eye clock --json         # the same as GET /api/clock
eye clock detect         # judge the prompts that were not judged yet, now
```

## What it measures

Everything comes from the turn events eye keeps for each session (see `architecture.md`, `turns/`):
timings and sizes, never text.

| Side | Signal | Where it comes from |
|---|---|---|
| You | prompts per local hour | the time of each prompt |
| You | **corrections** (pushback on something the agent did wrong) | a model, from the prompt and the end of the reply before it |
| You | interrupts | you stopping the agent while it works |
| Model | failed tool calls per call | tool results that report an error |
| Model | thinking depth | the agent's own measure of each thinking block, compared only with the same agent's other values |

A **break** is a gap of `[clock] break_minutes` (45) or more between prompts, across all your
agents, **or** (with `[clock] idle = true`) a stretch of that many minutes with no keyboard or mouse
input (see below). Both count: the daemon only samples idle time while it runs, so a night the
machine slept leaves no stretch, but the gap between the last prompt and the first one next morning
is still a break. The first prompt of the window starts the first stretch.

## How the verdict is made

- The 24 local hours each get a correction rate. An hour with at least 20 prompts and a rate of 1.5
  times the median hour's or more is a candidate for a bad hour.
- Your side counts only when random luck rarely does as well (the shuffle test below) and the
  candidate hours hold at least 10 corrections. Otherwise it is "not clearly different": the report
  never forces a pattern onto noise.
- The model's side is measured in the same hours, **per agent, never pooled** (agents fail at
  different rates, so a week that mixes them would compare the agents, not the hours):
  - Failed tool calls per call: each agent's rate in the hours against its own rate elsewhere. It
    moved when the ratio is outside 0.8 to 1.25 **and** the same two-proportion test (under one in
    ten, at least 10 failed calls in the higher group) says chance does not explain it.
  - Thinking: each agent's median in the hours against its own median elsewhere, only with at least
    30 thoughts inside the hours and 30 outside, the agents' values pooled. It moved when the ratio
    is outside 0.9 to 1.1 **and** the shuffle test below says luck rarely does as well. A ratio
    outside the range that luck explains reads "not clearly different".
  - A signal with too little data is not measured. When nothing on the model's side was measured,
    the sentence says "no model-side data" and never "flat".
- `you`: your side passes the test and the model's side was measured and did not move. `model`: the
  model's side moved and yours is not clearly different. `both`: both moved. `unmeasured`: your side
  passes but nothing on the model's side was measured, so it cannot be said whose it was. `none`:
  enough data, nothing moves. `too_few`: under 300 prompts or 30 corrections in the window.
- The hours the sentence names are exactly the hours in `bad[]` (the chart draws them), for every
  verdict that names hours. Each has a `label` from one rule: **morning** 05:00 to 11:59,
  **afternoon** 12:00 to 16:59, **evening** 17:00 to 21:59, **night** 22:00 to 04:59, by the hour the
  run starts in. Several runs are named by their hours.
- The sentence is plain and has the counts, for example: *Evenings: 1.8x more corrections (105).
  Model thinking −8%. Tool errors flat. Mostly you.* When yours is not clearly different it says
  *your corrections not clearly different*.

### Why a shuffle test

The bad hours are picked *because* they look worst. Testing them afterwards as if someone had named
them in advance fools you: pick the worst-looking hours out of 24 hours of pure luck and they will
look bad, and a plain test will often agree. (On made-up weeks with no pattern at all, that plain
test named hours in more than half of them.)

So the test repeats the whole procedure on data where there is no pattern by construction:

1. Keep every prompt at its hour, and keep the total number of corrections.
2. Deal those corrections out again at random over the prompts, as if the hour of the day made no
   difference.
3. Run the same procedure on the dealt data: pick the hours at 1.5 times the median, and measure how
   much worse they are (a two-proportion z, 0 when nothing is picked).
4. Do this 1,000 times. The p-value is the share of dealings (counting your real data as one) that
   did as well as your real data. Your side counts when it is under 0.1.

Thinking is handled the same way: the thoughts inside and outside the hours are dealt out again at
random 300 times, and the medians compared each time. The seed is fixed, so two looks at the same
data give the same answer. A year of prompts takes about 4 ms and a year of thinking about 20 ms
(release build).

What it assumes: each prompt (and each thought) counts as independent. Prompts in one sitting are
related, so the test errs a little towards finding a pattern. In a seeded simulation of weeks with
no pattern (`clock::permute`'s tests) hours were named in about 9% of them; with a real 2x effect in
three evening hours of 3,600 prompts they were found in nearly all.

The thinking measure is only a proxy (for Claude Code it is the length of the thinking signature),
and corrections are a model's judgement, wrong about one time in ten. Read the verdict as a hint
about your own habits, not a score.

## What is sent where

Judging corrections is the only step that sends anything, and it goes through the agent you already
use for summaries, with the same call. What that call can and cannot do depends on the agent
(`security.md`, "What is sent to the agent", has the command lines):

| Agent | Tools | Session kept | Notes |
|---|---|---|---|
| Claude Code | none (`--tools ""`), no MCP servers, hooks off | none (`--no-session-persistence`) | |
| Codex CLI | no tool-less mode: a read-only sandbox, so it can read files but not write or run anything that changes them, and the sandbox has no network | none (`--ephemeral`) | your config, hooks and rules are ignored |
| agy | plan mode and `--sandbox`: agy's own changelog says plan mode does not edit or run commands; not tested here | the conversation is deleted after the call: its folder, database and row in `conversation_summaries.db`; `history.jsonl` is only edited while agy is closed | the run happens in a folder named `eye-ask-<uuid>`, which the scanner skips |
| OpenCode | every permission is denied by default and every tool is off, for the run (shell, edits, web fetches and search, sub-agents, skills and your MCP servers' tools). Written from OpenCode's documentation, not proven on a real install | the session is deleted after the call | **skipped by the pass** |

The pass only asks an agent whose locked-down run is proven. Claude Code, Codex and agy are.
**OpenCode is skipped**: its rules were written from its documentation and have not been checked on
a real install (`security.md`), so with OpenCode as the summary agent the pass sends nothing, and
`judging` is `unavailable` with `judging_reason` saying so. It stays skipped until a real-run check
exists.

- A word filter first: only prompts of 1,500 characters or fewer that contain one of *no, not,
  don't, never, stop, again, still, why, wrong, instead, too, shorter, simpler, concise, plain,
  jargon, remove, less, should, must, I said, we agreed, please, avoid* after a reply are sent.
- Each is sent with the last 700 characters of the reply before it, masked for keys and tokens as
  summary excerpts are, in batches of up to 100.
- Sessions in excluded folders are never sent.
- The daemon does this once a night (at 04:00 or later, or inside the night window when the night
  shift is on), newest first, at most 300 exchanges. Older prompts follow on later nights, and the
  report's `pending` says how many are still waiting. `eye clock detect` does a pass now.
- Prompts of sessions started by a script (`claude -p`, `codex exec`) or by the night shift are not
  sent and not counted: they are not you steering.
- The nightly pass does not start while you are at work, as summaries do not: any turn in the last
  10 minutes (your prompt, or an agent's reply or tool call, so a long run you started counts while
  it works), or (with the idle sensor on) input in the last 2. It stays due and starts when you
  stop, later that night or the next.
- It stops quietly when the agent is missing or its plan limit is hit, and tries again the next
  night. Each exchange is judged once and stored with a hash of what was judged; the same exchange in
  a forked session reuses the verdict.
- The model is the one summaries use (`[summary] model`, else the agent's cheap model). On a private
  set of 40 labelled prompts, Haiku found 26 of 26 corrections with no false alarms, and Sonnet
  24 of 26.

## The idle sensor

`[clock] idle = true` (off by default) makes the daemon ask the system, once a minute, how long it
has been since the last input. It reads that one number:

- macOS: `HIDIdleTime` from `ioreg -c IOHIDSystem -d 4`.
- Linux: your graphical session is found through logind (`loginctl show-user`, its `Display`), not
  through `XDG_SESSION_ID`, which a systemd user service does not have. Its `IdleHint` and
  `IdleSinceHint` come from `loginctl show-session`, where the desktop reports them.
- Anywhere else the sensor is unavailable, and breaks stay on prompt gaps.

It never sees which keys, which window or which application. Stretches of 5 minutes or more are
stored (a start and an end) in `idle_span`. `eye doctor` says whether the daemon itself can read the
idle time (the daemon notes it in the database once it tries; a terminal's own reading is only used
while the daemon is not running) and whether the sensor is on.

## Switching it off

- `[clock] llm = false`: nothing is ever sent; corrections stay at zero and the verdict says so.
  `/api/clock` says why corrections are zero: `judging` is `on`, `off` or `unavailable` (no enabled
  agent can judge, or the summary agent is OpenCode), with a short `judging_reason`. The verdict text
  uses the same reason, never `[clock] llm` when something else is why.
- `[clock] enabled = false`: no report and no pass. What is stored stays; `eye forget` and
  `eye uninstall --purge` remove it with the rest. The daemon records turns by the setting it
  started with, so recording stops at its next start (the gear says so).
- `[clock] idle` is already off unless you turned it on.

Changes to `[clock]` made in the gear or in `config.toml` count without a restart: the report, the
idle sampling and the nightly pass read the file again.

`[clock] hints = true` fills `now.hint` of `/api/clock` (`{ text, offer }`) while you are in a bad
hour, for a board that chooses to show it. The hint is always worked out from the last 4 weeks,
whatever `weeks` a report was asked for, so the board, `eye clock --brief` and the picker say the
same thing at the same moment. `offer` is true when the night shift is on, so a surface
with a key or a button for it can show it next to `text`. The board shows it as a mark beside HOURS and on the card you are working in, and puts
Continue tonight first in that card's menu. The picker shows it after the keys line when the screen is wide
enough. Nothing pops up.

## Other agents

Prompts, corrections and breaks work for every agent eye follows. The model's side (thinking, tool
errors) depends on what the agent's files record. `GET /api/clock` lists, per agent, its prompts,
its corrections, its thinking measure (`think_unit`, null when it has none), its model side in the
bad hours against its own other hours (`think_ratio`, `tool_error_ratio`, null when it has no such
signal or too little of it) and which turn events it can report at all (`signals`; one it lacks is
"no signal", not zero). The hours' tool-call and failed-call counts are one agent's, `tool_agent`
(the one with the most tool calls), because the verdict compares each agent with itself; the
thinking strip is `think_agent`'s. `eye clock` prints one line per agent with a dash for a missing signal
(`agents.md`, "Turn events by agent").
