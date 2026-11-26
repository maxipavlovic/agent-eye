import { describe, expect, it } from 'vitest';
import { demoClock, emptyClock } from '../data/clockSample';
import { tooFewClock } from '../data/clockStates';
import { bandLabel, bandSpans, breakLine, clockNote, FAINT_BELOW, hourRange, inBand, modelLine, niceMax, partOfDay, pendingNote, thinkBaseline, thinkPercent, thinkSpan, toolLine, youLine } from './clockModel';

const hour = (over: Partial<ReturnType<typeof demoClock>['hours'][0]> = {}) => ({ h: 21, prompts: 108, corrections: 14, interrupts: 0, tool_calls: 900, tool_errors: 20, think: 1590, think_n: 800, ...over });

describe('the clock model', () => {
  it('names an hour from its start to the next', () => {
    expect(hourRange(21)).toBe('21:00–22:00');
    expect(hourRange(23)).toBe('23:00–24:00');
    expect(hourRange(5)).toBe('05:00–06:00');
  });

  it('knows which hours a band holds, also across midnight', () => {
    const b = { from_h: 18, to_h: 6 };
    expect([17, 18, 23, 0, 5, 6, 12].map((h) => inBand(h, b))).toEqual([false, true, true, true, true, false, false]);
    expect(inBand(9, { from_h: 9, to_h: 12 })).toBe(true);
    expect(inBand(12, { from_h: 9, to_h: 12 })).toBe(false);
    expect(inBand(3, { from_h: 4, to_h: 4 })).toBe(false);
  });

  it('draws a band across midnight as two pieces and labels the first', () => {
    expect(bandSpans({ from_h: 18, to_h: 6 })).toEqual([{ from: 18, to: 24, label: true }, { from: 0, to: 6, label: false }]);
    expect(bandSpans({ from_h: 20, to_h: 0 })).toEqual([{ from: 20, to: 24, label: true }]);
    expect(bandSpans({ from_h: 9, to_h: 12 })).toEqual([{ from: 9, to: 12, label: true }]);
    expect(bandSpans({ from_h: 5, to_h: 5 })).toEqual([]);
  });

  it('labels a band by the part of the day it starts in', () => {
    expect([6, 13, 18, 2].map(partOfDay)).toEqual(['morning', 'afternoon', 'evening', 'night']);
    expect(bandLabel({ from_h: 18, to_h: 6 })).toBe('your evening');
  });

  it('reads a busy hour as corrections per 100 prompts, and says when an hour is too thin', () => {
    expect(youLine(hour())).toBe('21:00–22:00 · 14 corrections in 108 prompts (13.0 per 100)');
    expect(youLine(hour({ prompts: FAINT_BELOW - 1, corrections: 1 }))).toContain('too few prompts');
    expect(youLine(hour({ prompts: FAINT_BELOW, corrections: 1 }))).not.toContain('too few');
    expect(youLine(hour({ prompts: 0, corrections: 0 }))).toBe('21:00–22:00 · no prompts');
    expect(youLine(hour({ prompts: 1, corrections: 1 }))).toContain('1 correction in 1 prompt ');
  });

  it('compares thinking with its median, hour by hour', () => {
    const hours = [100, 100, 80, null, 120].map((t, i) => hour({ h: i, think: t }));
    const base = thinkBaseline(hours);
    expect(base).toBe(100);
    expect(thinkPercent(hours[2]!, base)).toBe(80);
    expect(thinkPercent(hours[3]!, base)).toBeNull();
    expect(thinkSpan(hours, base)).toBe(25);
    expect(thinkSpan([hour({ think: 300 }), hour({ think: 100 })], 100)).toBe(200);
    expect(thinkBaseline([hour({ think: null })])).toBeNull();
    expect(modelLine(hours[2]!, base, 'signature')).toBe('thinking 80% of usual (thinking signature length, median of 800)');
    expect(modelLine(hours[3]!, base, null)).toBe('thinking: no data this hour');
  });

  it('reads tool errors per 100 calls', () => {
    expect(toolLine(hour())).toBe('20 failed in 900 calls (2.2 per 100)');
    expect(toolLine(hour({ tool_calls: 0, tool_errors: 0 }))).toBe('tool calls: none this hour');
  });

  it('reads a since-break bar', () => {
    expect(breakLine({ band: '2-4h', prompts: 200, corrections: 30 })).toBe('2-4h since a break · 30 corrections in 200 prompts (15.0 per 100)');
    expect(breakLine({ band: '4h+', prompts: 0, corrections: 0 })).toBe('4h+ since a break · no prompts');
  });

  it('rounds a scale up to a clean number', () => {
    expect([0, 3, 7.5, 12, 23, 40, 99, 140].map(niceMax)).toEqual([5, 5, 10, 20, 25, 50, 100, 200]);
  });

  it('says plainly when there is nothing to draw, and draws when there is', () => {
    expect(clockNote(demoClock(4))).toBeNull();
    expect(clockNote(tooFewClock())).toBe('Not enough data yet: 120 prompts, 6 corrections.');
    expect(clockNote(emptyClock(4))).toContain('Nothing recorded yet');
    expect(clockNote({ ...demoClock(4), enabled: false })).toContain('is off');
  });

  it('mentions prompts not judged yet only when there are some', () => {
    expect(pendingNote(demoClock(4))).toBe('12 prompts not judged yet: the nightly pass counts them.');
    expect(pendingNote({ ...demoClock(4), pending: 1 })).toBe('1 prompt not judged yet: the nightly pass counts them.');
    expect(pendingNote({ ...demoClock(4), pending: 0 })).toBeNull();
  });
});
