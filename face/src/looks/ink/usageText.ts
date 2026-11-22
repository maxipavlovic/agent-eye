import type { UsageLimitWindow, UsageSum } from '../../api/types';
import { tok } from '../../format';

/** A limit above this share of the window is shown in the accent colour; the rest stay ink. */
export const HOT_PERCENT = 80;

/** "$12.40", "$12.40 + unpriced" (some tokens have no price), "unknown" (none has). */
export function costText(s: UsageSum): string {
  if (s.total === 0) return '$0.00';
  if (s.cost_usd === null) return 'unknown';
  const c = s.cost_usd >= 100 ? `$${Math.round(s.cost_usd).toLocaleString('en-GB')}` : `$${s.cost_usd.toFixed(2)}`;
  return s.unpriced_tokens > 0 ? `${c} + unpriced` : c;
}

/** "4.2M tokens". */
export function tokensText(s: UsageSum): string {
  return `${tok(s.total)} tokens`;
}

/** The number a bar is drawn from: the cost, or the tokens when no row in the group has a cost. */
export function barValue(s: UsageSum, byCost: boolean): number {
  return byCost ? s.cost_usd ?? 0 : s.total;
}

/** Bars scale to the largest row; a row that is not zero never shrinks below a sliver. */
export function barPercents(rows: UsageSum[]): number[] {
  const byCost = rows.some((r) => r.cost_usd !== null && r.cost_usd > 0);
  const max = Math.max(0, ...rows.map((r) => barValue(r, byCost)));
  return rows.map((r) => {
    const v = barValue(r, byCost);
    return max <= 0 || v <= 0 ? 0 : Math.max(2, Math.round((v / max) * 100));
  });
}

export function limitHot(w: UsageLimitWindow): boolean {
  return !w.reset && w.used_percent > HOT_PERCENT;
}

/** "23%" or "23.5%". */
export function percentText(p: number): string {
  return Math.abs(p - Math.round(p)) < 0.05 ? `${Math.round(p)}%` : `${p.toFixed(1)}%`;
}

/** "Mon 09:00", in the browser's time zone. */
export function resetText(ms: number): string {
  const d = new Date(ms);
  const day = d.toLocaleDateString('en-GB', { weekday: 'short' });
  return `${day} ${d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })}`;
}

/** "just now", "12 min ago", "3 h ago", "2 d ago". */
export function seenText(ms: number, now = Date.now()): string {
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}
