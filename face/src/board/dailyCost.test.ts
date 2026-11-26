import { describe, expect, it } from 'vitest';
import type { UsageSeries } from '../api/types';
import { costMax, dayCost, dayLabel, dayTotal, dollars, keyCostText, noCost } from './dailyCost';

const series: UsageSeries = {
  by: 'model', keys: ['opus', 'mystery'],
  days: [
    { day: '2026-11-20', cost: { opus: 12.5 }, tokens: { opus: 100, mystery: 10 } },
    { day: '2026-11-21', cost: {}, tokens: {} },
  ],
};

describe('usage by day', () => {
  it('reads a key that has tokens and no price as unknown, and an absent key as zero', () => {
    const d = series.days[0]!;
    expect(dayCost(d, 'opus')).toBe(12.5);
    expect(dayCost(d, 'mystery')).toBeNull();
    expect(dayCost(series.days[1]!, 'opus')).toBe(0);
    expect(keyCostText(d, 'mystery')).toBe('no price for 10 tokens');
    expect(keyCostText(d, 'opus')).toBe('$12.50');
  });

  it('scales to the dearest day and says when no day cost anything', () => {
    expect(dayTotal(series.days[0]!)).toBe(12.5);
    expect(costMax(series)).toBe(20);
    expect(noCost(series)).toBe(false);
    expect(noCost({ ...series, days: [series.days[1]!] })).toBe(true);
    expect(costMax({ ...series, days: [] })).toBe(5);
  });

  it('writes dollars and days for people', () => {
    expect(dollars(5, true)).toBe('$5');
    expect(dollars(1500, true)).toBe('$1.5k');
    expect(dollars(1234.5)).toBe('$1,235');
    expect(dayLabel('2026-11-20')).toBe('Fri 20 Nov');
  });
});
