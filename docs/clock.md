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
agents. With `[clock] idle = true` it is a stretch of that many minutes with no keyboard or mouse
input instead (see below); prompts before the first such stretch have no known break and are left
out of the since-break rows.

## How the verdict is made

- The 24 local hours each get a correction rate. An hour with at least 20 prompts and a rate of 1.5
  times the median hour's or more is a candidate for a bad hour.
- The candidate hours are compared with the rest in a two-proportion test. They count as bad only
  when chance explains the difference less than one time in ten and they hold at least 10
  corrections. Otherwise there are no bad hours: the report never forces a pattern onto noise.
- The model's side is measured in the same hours: median thinking (each agent against its own
  median first) and failed tool calls per call, bad hours against the rest.
- `you`: your side passes the test and the model's is flat (thinking 0.9 to 1.1 of the rest, tool
  errors 0.8 to 1.25). `model`: the model's side moved and yours did not. `both`: both moved.
  `none`: enough data, nothing moves. `too_few`: under 300 prompts or 30 corrections in the window.
- The sentence is plain and has the counts, for example: *Evenings: 1.8x more corrections (105).
  Model thinking −8%. Tool errors flat. Mostly you.*

The thinking measure is only a proxy (for Claude Code it is the length of the thinking signature),
and corrections are a model's judgement, wrong about one time in ten. Read the verdict as a hint
about your own habits, not a score.

## What is sent where

Judging corrections is the only step that sends anything, and it goes through the agent you already
use for summaries, with the same call: no tools, no MCP servers, no hooks, no session kept
(`security.md`, "What is sent to the agent").

- A word filter first: only prompts of 1,500 characters or fewer that contain one of *no, not,
  don't, never, stop, again, still, why, wrong, instead, too, shorter, simpler, concise, plain,
  jargon, remove, less, should, must, I said, we agreed, please, avoid* after a reply are sent.
- Each is sent with the last 700 characters of the reply before it, masked for keys and tokens as
  summary excerpts are, in batches of up to 100.
- Sessions in excluded folders are never sent.
- The daemon does this once a night (at 04:00 or later, or inside the night window when the night
  shift is on), newest first, at most 300 exchanges. Older prompts follow on later nights, and the
  report's `pending` says how many are still waiting. `eye clock detect` does a pass now.
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
- Linux: `IdleHint` and `IdleSinceHint` of your session from `loginctl show-session`, where the
  desktop reports them.
- Anywhere else the sensor is unavailable, and breaks stay on prompt gaps.

It never sees which keys, which window or which application. Stretches of 5 minutes or more are
stored (a start and an end) in `idle_span`. `eye doctor` says whether the sensor is available and
on.

## Switching it off

- `[clock] llm = false`: nothing is ever sent; corrections stay at zero and the verdict says so.
- `[clock] enabled = false`: no recording and no report. What is stored stays; `eye forget` and
  `eye uninstall --purge` remove it with the rest.
- `[clock] idle` is already off unless you turned it on.

`[clock] hints = true` fills `now.hint` of `/api/clock` while you are in a bad hour, for a board that
chooses to show it. Nothing pops up in the terminal.

## Other agents

Prompts, corrections and breaks work for every agent eye follows. The model's side (thinking, tool
errors) depends on what the agent's files record; `GET /api/clock` lists, per agent, its prompts,
its corrections and the unit of its thinking measure, null when it has none (`agents.md`).
