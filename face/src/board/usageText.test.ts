import { describe, expect, it } from 'vitest';
import type { UsageSum } from '../api/types';
import { barPercents, costText, limitHot, percentText, seenText, tokensText } from './usageText';

const sum = (total: number, cost: number | null, unpriced = 0): UsageSum => ({ input: total, output: 0, cache_write: 0, cache_read: 0, total, cost_usd: cost, unpriced_tokens: unpriced });

describe('usage text', () => {
  it('says the cost, or that it is unknown, never a guess', () => {
    expect(costText(sum(0, 0))).toBe('$0.00');
    expect(costText(sum(10, 3.456))).toBe('$3.46');
    expect(costText(sum(10, 1234.5))).toBe('$1,235');
    expect(costText(sum(10, null, 10))).toBe('unknown');
    expect(costText(sum(10, 2, 4))).toBe('$2.00 + unpriced');
    expect(tokensText(sum(4_200_000, 1))).toBe('4.2M tokens');
  });

  it('scales bars to the largest row, by cost, or by tokens when no row has a cost', () => {
    expect(barPercents([sum(10, 10), sum(10, 5), sum(10, 0.01), sum(10, null, 10)])).toEqual([100, 50, 2, 0]);
    expect(barPercents([sum(200, null, 200), sum(50, null, 50)])).toEqual([100, 25]);
    expect(barPercents([])).toEqual([]);
    expect(barPercents([sum(0, 0)])).toEqual([0]);
  });

  it('marks a limit above 80 percent, unless its window has reset', () => {
    const w = (used_percent: number, reset = false) => ({ label: 'weekly', used_percent, window_minutes: 10_080, resets_at: 1, reset });
    expect(limitHot(w(80))).toBe(false);
    expect(limitHot(w(80.5))).toBe(true);
    expect(limitHot(w(95, true))).toBe(false);
    expect(percentText(23)).toBe('23%');
    expect(percentText(23.46)).toBe('23.5%');
  });

  it('gives the age of a reading in words', () => {
    const now = 10_000_000;
    expect(seenText(now - 5_000, now)).toBe('just now');
    expect(seenText(now - 12 * 60_000, now)).toBe('12 min ago');
    expect(seenText(now - 3 * 3_600_000, now)).toBe('3 h ago');
    expect(seenText(now - 2 * 86_400_000, now)).toBe('2 d ago');
  });
});
