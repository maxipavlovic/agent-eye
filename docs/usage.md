# Usage, cost and plan limits

`eye usage` answers "what did my sessions use and what would that cost" by day, project, agent, model or session, and shows where your plan limits stand. The board has the same in its USAGE panel.

**Every cost here is an API-equivalent estimate, not your bill.** It is the tokens a session used, priced at the vendors' list prices for pay-per-token API use. A subscription is a flat fee, so what you pay does not move with these numbers; they tell you which work was heavy, and what it would have cost on the API.

## The command

```sh
eye usage                      # the last 7 days, by project
eye usage --month --by agent   # the last 30 days
eye usage --days 14 --by model
eye usage --week --by session  # top sessions first
eye usage --week --json        # the same data the board gets
```

`--days N`, `--week` (7) and `--month` (30) count local days, today included. `--by` is `project` (the repository or folder), `agent`, `model` or `session`. Rows are sorted by estimated cost, then by tokens; the last row is the total. Columns are input, output and cache tokens (cache writes plus cache reads) and the estimated cost.

The plan limits come first, one line per window, with when they were seen:

```
Codex: 23% of the weekly limit, resets Mon 09:00 (seen 12 min ago)
Claude: 41% of the 5-hour limit, resets 18:00 (seen 3 min ago)
```

A window whose time has passed reads "window has reset since": the percentage is old news. Nothing is guessed: a limit is shown only as the agent last reported it.

`eye show <id>` and the picker's preview show the tokens and the estimate of one session.

## Where the numbers come from

Each agent's adapter reports the token use of every assistant message with its time and model (`Agent::usage_msgs` for a whole transcript, `Delta::usage_msgs` as the scanner reads it). The core adds them into local days per model in `usage_day` (schema 15). A session's sub-agents count toward the session, like `tok_sub`.

- **Claude Code:** `message.usage`, `message.model` and `timestamp` of each assistant line; a message written on several lines counts once; sub-agent transcripts (`<session>/subagents/*.jsonl`) are added.
- **Antigravity CLI:** the token counts of each model call in the conversation's JSON log, on the model named in its database.
- **OpenCode:** the `tokens` of each assistant message (input, output, cache read and write), on its `modelID`.
- **Codex:** the growth of the thread total between `token_count` events, on the model of the last `turn_context`. A total that goes down (the thread was rewound) starts again and adds nothing.

Sessions eye read before this existed are counted once in the background: the daemon does a few per tick (newest first), `eye scan` and `eye usage` do all of them. It reads the transcript, or eye's archive copy when the original is gone, and touches no read cursor. When it is done, `meta` has `usage_backfilled_at`. Until then the board says how many sessions are left. A rewritten transcript is counted again from the files. The sum by day equals the session's token columns plus `tok_sub`, which `eye usage` and the tests rely on.

Days are local days at the time of each message, worked out when the message is stored; a later change of time zone does not move what is stored.

## Prices

All prices are in `src/usage/prices.rs`, in USD per million tokens, with the date they were checked (2026-11-21) and one source per vendor:

- Claude: <https://platform.claude.com/docs/en/about-claude/pricing> (base input, output, 5-minute cache write, cache hit). Claude 5.5/5.1/5, Fable, Mythos, Opus 4.x, Sonnet 4.x and Haiku 4.5/5.5 are in the table. Haiku 5.5 is priced for prompts up to 100,000 tokens; a longer prompt costs more on the API and the daily totals cannot tell, so its cost is a floor.
- OpenAI (Codex models): <https://developers.openai.com/api/docs/pricing> (standard tier, prompts up to 272K tokens). Codex counts cached input inside input; eye splits it, so cached input is priced at the cached rate.

A model with no price (a new one, a model not in the table) shows its tokens and `unknown` as the cost, or `$12.40 + unpriced` when only part of a group has no price. Eye never guesses. Add or fix a model in `config.toml`:

```toml
[usage.prices."my-model"]
input = 3.0
output = 15.0
cache_write = 3.75
cache_read = 0.30
```

A model named there is priced from that table alone. See `config.md`.

## Plan limits

- **Codex** writes `rate_limits` (`primary` and `secondary`: `used_percent`, `window_minutes`, `resets_at`) in each rollout's `token_count` events. The scanner keeps the newest one it has seen in `meta` as `limits:codex`, with the time of the event.
- **Claude Code** gives limits to the status line command. The official status line page (<https://code.claude.com/docs/en/statusline>) says the JSON has `rate_limits.five_hour` and `rate_limits.seven_day`, each with `used_percentage` (0 to 100) and `resets_at` (Unix seconds); "`rate_limits`: appears only for claude.ai Pro, Max, and Ultra subscribers ... Even then, it appears only after the first API response in the session. Each window may be independently absent". There is nothing for API-key users. Transcripts do not carry limits, so eye can only see them through the status line.

### Turning the Claude limits on

`eye install` does **not** change your status line. To opt in, set it in `~/.claude/settings.json` yourself.

No status line yet:

```json
{ "statusLine": { "type": "command", "command": "eye statusline" } }
```

It prints a short line (`5-hour 23% · weekly 41%`).

Keep your own status line and add eye's bookkeeping around it:

```json
{ "statusLine": { "type": "command", "command": "eye statusline --wrap '~/.claude/my-statusline.sh'" } }
```

`--wrap` runs your command through `sh -c` with the same JSON on its standard input and prints what it prints. `eye statusline` keeps the limits in the database (`limits:claude`, with the time seen), opens no config, runs no migration and waits at most 25 ms for the database. On any error (bad JSON, no database, a locked database, a failing wrapped command) it prints the wrapped output, or nothing; it never fails the status line. To turn it off, put your old command back.

Use the full path of the binary if `eye` is not on the PATH of the shell Claude Code uses.
