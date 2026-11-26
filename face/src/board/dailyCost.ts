import type { UsageDay, UsageSeries } from '../api/types';
import { niceMax } from './clockModel';

/** What one key used on one day, in dollars; null when its tokens that day have no known price. */
export function dayCost(d: UsageDay, key: string): number | null {
  const c = d.cost[key];
  if (c !== undefined) return c;
  return (d.tokens[key] ?? 0) > 0 ? null : 0;
}

/** Everything the keys cost on a day (tokens without a price add nothing). */
export function dayTotal(d: UsageDay): number {
  return Object.values(d.cost).reduce((a, b) => a + b, 0);
}

/** The chart's scale top: the dearest day rounded up to a clean number of dollars. */
export function costMax(s: UsageSeries): number {
  return niceMax(Math.max(0, ...s.days.map(dayTotal)));
}

/** "$12.40", or "$1.2k" from a thousand up, for an axis. */
export function dollars(n: number, axis = false): string {
  if (axis) return n >= 1000 ? `$${n / 1000}k` : `$${n}`;
  return n >= 1000 ? `$${Math.round(n).toLocaleString('en-GB')}` : `$${n.toFixed(2)}`;
}

/** "Fri 20 Nov". */
export function dayLabel(day: string): string {
  const d = new Date(`${day}T12:00:00Z`);
  return `${d.toLocaleDateString('en-GB', { weekday: 'short', timeZone: 'UTC' })} ${d.getUTCDate()} ${d.toLocaleDateString('en-GB', { month: 'short', timeZone: 'UTC' })}`;
}

/** The words of a tooltip line for one key on one day: "$12.40", or "tokens without a price". */
export function keyCostText(d: UsageDay, key: string): string {
  const c = dayCost(d, key);
  if (c === null) return `no price for ${d.tokens[key]!.toLocaleString('en-GB')} tokens`;
  return dollars(c);
}

/** Every day in the window has no cost at all: the chart says so instead of drawing nothing. */
export const noCost = (s: UsageSeries) => s.days.every((d) => dayTotal(d) === 0);
