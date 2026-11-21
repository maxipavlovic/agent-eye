import type { UsageReport, UsageRow, UsageSum, UsageWeek } from '../api/types';

const ESTIMATE = 'API-equivalent estimate, not your bill';

function sum(input: number, output: number, cache: number, cost: number | null): UsageSum {
  const cache_write = Math.round(cache * 0.05);
  return { input, output, cache_write, cache_read: cache - cache_write, total: input + output + cache, cost_usd: cost, unpriced_tokens: cost === null ? input + output + cache : 0 };
}

const row = (name: string, s: UsageSum, extra: Partial<UsageRow> = {}): UsageRow => ({ name, ...s, ...extra });

/** No usage yet: a new install, or the plain sample. */
export function emptyUsage(days: number): UsageReport {
  const zero = sum(0, 0, 0, 0);
  const weeks: UsageWeek[] = [0, 1, 2, 3].map((i) => ({ from: `2026-11-${String(2 + i * 7).padStart(2, '0')}`, to: `2026-11-${String(8 + i * 7).padStart(2, '0')}`, ...zero }));
  return { estimate: ESTIMATE, days, from: '2026-11-15', to: '2026-11-21', by: 'project', rows: [], totals: zero, by_project: [], by_agent: [], by_model: [], top_sessions: [], weeks, limits: [], backfill_left: 0 };
}

/** The demo board's usage: a few projects, two agents, four weeks, and a Codex limit near the top. */
export function demoUsage(days: number): UsageReport {
  const projects = [
    row('~/code/billing', sum(9_000, 2_400_000, 610_000_000, 142.6)),
    row('~/code/docs', sum(4_000, 900_000, 190_000_000, 51.3)),
    row('~/code/site', sum(2_000, 310_000, 70_000_000, 18.9)),
    row('~/code/scratch', sum(600, 40_000, 8_000_000, 2.1)),
  ];
  const agents = [row('claude', sum(14_000, 3_500_000, 860_000_000, 214.7)), row('codex', sum(1_600, 150_000, 18_000_000, 0.2))];
  const sessions = ['Fix the invoice rounding', 'Rewrite the install page', 'Plan the 0.4 release', 'Speed up the picker', 'Trim the changelog']
    .map((t, i) => row(t, sum(2_000 - i * 300, 600_000 - i * 90_000, 140_000_000 - i * 22_000_000, 40 - i * 6), { id: `a000000${i + 1}-0000-4000-8000-00000000000${i + 1}`, agent: i === 3 ? 'codex' : 'claude' }));
  const weeks: UsageWeek[] = [
    { from: '2026-11-02', to: '2026-11-08', ...sum(6_000, 1_900_000, 430_000_000, 101.4) },
    { from: '2026-11-09', to: '2026-11-15', ...sum(9_000, 2_700_000, 640_000_000, 158.0) },
    { from: '2026-11-16', to: '2026-11-22', ...sum(11_000, 3_000_000, 700_000_000, 176.2) },
    { from: '2026-11-23', to: '2026-11-29', ...sum(15_600, 3_650_000, 878_000_000, 214.9) },
  ];
  const now = Date.now();
  return {
    estimate: ESTIMATE, days, from: '2026-11-23', to: '2026-11-29', by: 'project',
    rows: projects, totals: sum(15_600, 3_650_000, 878_000_000, 214.9),
    by_project: projects, by_agent: agents, by_model: [], top_sessions: sessions, weeks,
    limits: [
      { agent: 'codex', name: 'Codex', seen_at: now - 12 * 60_000, plan: 'plus', windows: [{ label: 'weekly', used_percent: 86, window_minutes: 10_080, resets_at: now + 2 * 86_400_000, reset: false }], lines: ['Codex: 86% of the weekly limit, resets Mon 09:00 (seen 12 min ago)'] },
      { agent: 'claude', name: 'Claude', seen_at: now - 3 * 60_000, plan: null, windows: [{ label: '5-hour', used_percent: 23, window_minutes: 300, resets_at: now + 3 * 3_600_000, reset: false }, { label: 'weekly', used_percent: 41, window_minutes: 10_080, resets_at: now + 4 * 86_400_000, reset: false }], lines: ['Claude: 23% of the 5-hour limit, resets 18:00 (seen 3 min ago)', 'Claude: 41% of the weekly limit, resets Fri 09:00 (seen 3 min ago)'] },
    ],
    backfill_left: 0,
  };
}
