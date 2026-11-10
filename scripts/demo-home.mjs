#!/usr/bin/env node
// Builds a made-up Claude Code folder for demos and screen recordings: transcripts for a dozen
// invented sessions, plus a stand-in `claude` command that answers the summary calls.
// Nothing here is real data. Usage: node scripts/demo-home.mjs <dir> [live-pid]
//   <dir>/claude        the fake ~/.claude  (use with EYE_CLAUDE_DIR)
//   <dir>/home          a fake home with the session folders (use as HOME)
//   <dir>/bin/claude    the stand-in command (put <dir>/bin first on PATH)
//   live-pid            a running process id; the "dark mode" session shows as open and busy
import { chmodSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';

const dir = resolve(process.argv[2] ?? '');
const livePid = Number(process.argv[3] ?? 0);
if (!process.argv[2]) {
  console.error('usage: node scripts/demo-home.mjs <dir> [live-pid]');
  process.exit(2);
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const now = Date.now();
// The made-up home folder. The session folders must exist, or resume refuses to run.
const HOME = join(dir, 'home');

// age: how long ago the last message was. turns: [prompt, reply, tools?]. say: the stand-in
// summary (status, state, next). pr: a pull request link that shows up in a tool result.
const SESSIONS = [
  {
    key: 'webhook', title: 'Retry failed webhook deliveries', project: 'payments-api', branch: 'fix/webhook-retries',
    age: 12 * MIN,
    turns: [
      ['Webhook deliveries to merchants fail for a few minutes after a deploy and are never retried. Can we retry them?',
       'Right now a failed delivery is logged and dropped. I will add a retry queue with exponential backoff and a dead-letter table.'],
      ['Go ahead, keep it small.', 'Added `retry_delivery()` with backoff (30 s, 2 min, 10 min) and a `webhook_dead_letter` table. 14 tests pass.'],
      ['Nice. What about merchants that are down for hours?',
       'After the last attempt the delivery goes to the dead-letter table and shows in the dashboard. How many attempts should we allow before that: 5 or 8?'],
    ],
    say: ['needs_you', 'Retry queue with backoff and a dead-letter table are in; 14 tests pass', 'You: choose the max attempts, 5 or 8'],
  },
  {
    key: 'dark mode', title: 'Add dark mode to the settings page', project: 'web-shop', branch: 'feature/dark-mode',
    age: 40 * MIN, live: true,
    turns: [
      ['Add a dark theme to the shop settings page. Use the existing design tokens.',
       'The tokens already have dark values. I will wire a `data-theme` switch and convert the settings page first.'],
      ['Sounds good. Keep the contrast above AA.', 'Theme switch is in. Converting the profile and address forms now, the billing form is next.'],
    ],
    say: ['paused', 'Theme switch and tokens are in; profile and address forms converted', 'Claude: convert the billing form, then check contrast'],
  },
  {
    key: 'invoices', title: 'Move invoices to the new queue', project: 'payments-api', branch: 'feature/invoice-queue',
    age: 3 * HOUR, pr: 'https://github.com/acme/payments-api/pull/412',
    turns: [
      ['Invoice generation still runs inside the request. Move it to the new job queue.', 'I will add an `InvoiceJob`, enqueue it from the checkout handler and keep the old path behind a flag.'],
      ['Open a PR when the tests pass.', 'Tests pass. Opened PR #412 with the queue change and a rollout note.'],
    ],
    say: ['waiting_on_others', 'PR #412 opened; invoice jobs run on the queue behind a flag', 'Dana: review PR #412'],
  },
  {
    key: 'flaky', title: 'Fix the flaky checkout test', project: 'web-shop', branch: 'bugfix/flaky-checkout',
    age: 5 * HOUR,
    turns: [
      ['`checkout.spec` fails one run in ten on CI. Find out why.', 'It fails when the day rolls over during the test: the coupon expiry uses the real clock. The fixture never freezes time.'],
      ['Ok, so freeze it.', 'Not written yet; I stopped before editing the fixture.'],
    ],
    say: ['paused', 'Cause found: the coupon test uses the real clock; no fix written yet', 'Claude: freeze the clock in the checkout fixture'],
  },
  {
    key: 'queues', title: 'Write the Terraform module for queues', project: 'infra', branch: 'main',
    age: 22 * HOUR,
    turns: [
      ['We need a Terraform module for the job queues: one queue, one dead-letter queue, alarms.', 'Drafted `modules/queue` with the queue, the dead-letter queue and two alarms. Variables: `name`, `max_receive_count`, `alarm_topic`.'],
      ['Does it need an access policy?', 'Yes. Which service accounts should be allowed to send and receive? I need their names to write the policy.'],
    ],
    say: ['needs_you', 'Module drafted with queue, dead-letter queue and alarms; access policy missing', 'You: tell me which service accounts may send and receive'],
  },
  {
    key: 'postgres', title: 'Upgrade the Postgres client to 17', project: 'payments-api', branch: 'chore/pg17-client',
    age: 2 * DAY, pr: 'https://github.com/acme/payments-api/pull/398',
    turns: [
      ['Bump the Postgres client library to the 17 line.', 'Bumped. Two deprecated calls needed small changes; the test suite passes.'],
      ['Merge it when CI is green.', 'CI is green and PR #398 is merged.'],
    ],
    say: ['done', 'Client upgraded to 17, CI green, merged in PR #398', ''],
  },
  {
    key: 'release notes', title: 'Draft the release notes for 2.4', project: 'docs-site', branch: 'docs/release-2-4',
    age: 2 * DAY + 3 * HOUR,
    turns: [
      ['Draft release notes for 2.4 from the merged PR titles since 2.3.', 'Listed 23 changes in four groups: new, improved, fixed, removed. I flagged two titles that were unclear.'],
      ['Looks right. Save it as docs/releases/2.4.md.', 'Saved `docs/releases/2.4.md`.'],
    ],
    say: ['done', 'Release notes for 2.4 are saved in docs/releases/2.4.md', ''],
  },
  {
    key: 'slow search', title: 'Investigate slow search on the catalog page', project: 'web-shop', branch: 'main',
    age: 3 * DAY,
    turns: [
      ['Search on the catalog page takes 4 seconds for common words. Why?', 'The query scans `products` with a `LIKE` on three columns. The plan shows a sequential scan over 1.2M rows.'],
      ['Write it up for the database team.', 'Wrote the profile and the proposed index into `notes/search-index.md` and sent it to the database team.'],
    ],
    say: ['waiting_on_others', 'Profile sent: sequential scan on products; an index is proposed', 'Priya: add the index on products(name, brand)'],
  },
  {
    key: 'jenkins', title: 'Move the CI from Jenkins to Actions', project: 'infra', branch: 'ci/actions',
    age: 4 * DAY,
    turns: [
      ['Port the build and test jobs from Jenkins to GitHub Actions.', 'Ported build and unit tests. The integration job needs a database service; I left it out for now.'],
    ],
    say: ['paused', 'Build and unit tests run on Actions; the integration job is not ported', 'Claude: add the database service and port the integration job'],
  },
  {
    key: 'onboarding', title: 'Onboarding screens for the mobile app', project: 'mobile-app', branch: 'feature/onboarding',
    age: 6 * DAY,
    turns: [
      ['Sketch three layouts for the onboarding flow: welcome, permissions, first task.', 'Here are three layouts: a single scroll, a three-step pager and a checklist. The pager needs the least code; the checklist tests best in other apps.'],
      ['Let me think.', 'Take your time. Tell me which one to build and I will start.'],
    ],
    say: ['needs_you', 'Three onboarding layouts drafted: scroll, pager and checklist', 'You: pick a layout'],
  },
  {
    key: 'tenant', title: "Rename 'tenant' to 'workspace'", project: 'web-shop', branch: 'refactor/workspace',
    age: 9 * DAY,
    turns: [
      ['Rename the tenant concept to workspace in code, API and docs.', 'Renamed in 61 files, kept `/tenants` as a deprecated alias, and updated the docs.'],
    ],
    say: ['done', "Renamed tenant to workspace; the old API path stays as an alias", ''],
  },
  {
    key: 'rate limit', title: 'Add rate limit headers to the API', project: 'payments-api', branch: 'feature/rate-headers',
    age: 11 * DAY, pr: 'https://github.com/acme/payments-api/pull/377',
    turns: [
      ['Return `X-RateLimit-Limit`, `-Remaining` and `-Reset` on every API response.', 'Added a middleware that sets the three headers from the limiter state, with tests for the 429 path.'],
    ],
    say: ['done', 'Rate limit headers ship on all responses; merged in PR #377', ''],
  },
  {
    key: 'export job', title: 'Why does the export job time out?', project: 'data-jobs', branch: 'main',
    age: 14 * DAY,
    turns: [
      ['The nightly export job times out after 30 minutes. Look at it.', 'It loads every row into memory before writing. Streaming in batches of 5,000 should cut the time to a few minutes.'],
    ],
    say: ['paused', 'Cause found: the export loads all rows into memory; batching is the fix', 'Claude: stream the export in batches of 5,000'],
  },
];

const iso = (ms) => new Date(ms).toISOString();
const line = (o) => JSON.stringify(o) + '\n';

function transcript(s, id) {
  const cwd = `${HOME}/code/${s.project}`;
  const turns = s.turns;
  const span = Math.min(s.age * 0.6, 3 * HOUR);
  const step = span / (turns.length * 2 + 2);
  let t = now - s.age - span;
  let out = line({ type: 'custom-title', customTitle: s.title, sessionId: id });
  let parent = null;
  const base = (type, ts) => ({ parentUuid: parent, isSidechain: false, type, timestamp: iso(ts), cwd, sessionId: id, gitBranch: s.branch });
  turns.forEach(([prompt, reply], i) => {
    t += step;
    const u = randomUUID();
    out += line({ ...base('user', t), message: { role: 'user', content: prompt }, uuid: u });
    parent = u;
    t += step;
    const a = randomUUID();
    const content = [{ type: 'text', text: reply }];
    if (i === 0) content.push({ type: 'tool_use', id: `t${i}`, name: 'Bash', input: { command: 'git status --short' } });
    out += line({
      ...base('assistant', t),
      message: {
        id: `msg_${a.slice(0, 8)}`, role: 'assistant', content,
        usage: { input_tokens: 4200 + i * 900, output_tokens: 600 + i * 150, cache_read_input_tokens: 38000 + i * 5000 },
      },
      uuid: a,
    });
    parent = a;
    if (i === 0 && s.pr) {
      t += step / 4;
      const r = randomUUID();
      out += line({
        ...base('user', t),
        message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: `t${i}`, content: `Created pull request ${s.pr}` }] },
        uuid: r,
      });
      parent = r;
    }
  });
  return out;
}

rmSync(dir, { recursive: true, force: true });
const claude = join(dir, 'claude');
mkdirSync(join(claude, 'sessions'), { recursive: true });
mkdirSync(join(dir, 'bin'), { recursive: true });

const answers = [];
for (const s of SESSIONS) {
  const id = randomUUID();
  const proj = join(claude, 'projects', `${HOME}/code/${s.project}`.replace(/[/.]/g, '-'));
  mkdirSync(proj, { recursive: true });
  mkdirSync(`${HOME}/code/${s.project}`, { recursive: true });
  writeFileSync(join(proj, `${id}.jsonl`), transcript(s, id));
  if (s.live && livePid) {
    writeFileSync(
      join(claude, 'sessions', `${livePid}.json`),
      JSON.stringify({ pid: livePid, sessionId: id, cwd: `${HOME}/code/${s.project}`, status: 'busy', updatedAt: now - 20_000 }),
    );
  }
  const [status, state, next] = s.say;
  answers.push({ match: s.title, structured_output: { status, state, next, topics: [] } });
}

// The stand-in `claude`: finds the session's title in the excerpt and prints a summary envelope.
const cases = answers
  .map((a) => `  *"${a.match.replace(/(["\\$`])/g, '\\$1')}"*) cat <<'JSON'\n${JSON.stringify({ structured_output: a.structured_output, duration_ms: 1200, usage: {}, total_cost_usd: 0 })}\nJSON\n  ;;`)
  .join('\n');
const stub = `#!/bin/sh\n# Stand-in for the agent CLI in demos: answers summary calls with canned text.\nargs="$*"\ncase "$args" in\n${cases}\n  *) echo '{"is_error":true,"result":"no canned answer"}' ;;\nesac\n`;
writeFileSync(join(dir, 'bin', 'claude'), stub);
chmodSync(join(dir, 'bin', 'claude'), 0o755);
console.log(`demo home written to ${dir} (${SESSIONS.length} sessions)`);
