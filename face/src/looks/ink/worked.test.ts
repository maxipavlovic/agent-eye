// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';
import { h, render } from 'preact';
import type { CardDetail, Worked } from '../../api/types';
import { fixtureSource } from '../../data/fixture';
import { Where } from './Sections';
import { activeText, dayName, dayNumber, stripCells, workedText } from './worked';

const MIN = 60_000;
/** A local time on a local day, `back` days before `now`. */
const local = (now: Date, back: number, hh: number, mm: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate() - back, hh, mm).getTime();
const iso = (ms: number) => { const d = new Date(ms); return dayName(Math.round(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 86_400_000)); };

const worked = (o: Partial<Worked> = {}): Worked => ({
  first_day: '2026-11-10', last_day: '2026-11-13',
  days: [{ day: '2026-11-10', prompts: 3, active_ms: 100 * MIN }, { day: '2026-11-13', prompts: 1, active_ms: 10 * MIN }],
  days_count: 2, span_days: 4, active_ms: 110 * MIN, last_at: Date.now(), more_days: 0, ...o,
});

describe('the days worked, in words', () => {
  it('reads active time in hours and minutes', () => {
    expect(activeText(25 * MIN)).toBe('25 m');
    expect(activeText(460 * MIN)).toBe('7 h 40 m');
    expect(activeText(61 * MIN)).toBe('1 h 01 m');
    expect(activeText(5000)).toBe('under 1 m');
  });

  it('says days worked over the span, the time and when the last message was', () => {
    const now = new Date();
    const w = worked({ days_count: 6, span_days: 12, active_ms: 460 * MIN, last_at: local(now, 0, 14, 20) });
    expect(workedText(w, now.getTime())).toBe('6 days over 12 · 7 h 40 m active · last today 14:20');
    expect(workedText({ ...w, last_at: local(now, 1, 9, 5) }, now.getTime())).toBe('6 days over 12 · 7 h 40 m active · last yesterday 09:05');
  });

  it('says only the day and the time for a session worked on one day', () => {
    const now = new Date();
    const w = worked({ days_count: 1, span_days: 1, active_ms: 25 * MIN, last_at: local(now, 0, 9, 0) });
    expect(workedText(w, now.getTime())).toBe('today · 25 m active');
  });

  it('counts one worked day over several as "1 day"', () => {
    expect(workedText(worked({ days_count: 1, span_days: 3 }), Date.now())).toMatch(/^1 day over 3 /);
  });
});

describe('the strip of days', () => {
  it('has a cell for every calendar day, empty where there was no work, filled by activity', () => {
    const cells = stripCells(worked());
    expect(cells.map((c) => c.day)).toEqual(['2026-11-10', '2026-11-11', '2026-11-12', '2026-11-13']);
    expect(cells.map((c) => c.level)).toEqual([4, 0, 0, 1]);
    expect(cells[0]!.title).toContain('3 prompts');
    expect(cells[0]!.title).toContain('1 h 40 m active');
    expect(cells[1]!.title).toContain('no work');
  });

  it('gives a day with a message but no active time the lightest fill', () => {
    const w = worked({ days: [{ day: '2026-11-10', prompts: 1, active_ms: 0 }, { day: '2026-11-13', prompts: 1, active_ms: 0 }] });
    expect(stripCells(w).map((c) => c.level)).toEqual([1, 0, 0, 1]);
  });

  it('counts days across a month end', () => {
    expect(dayName(dayNumber('2026-02-28') + 1)).toBe('2026-03-01');
  });
});

describe('the WHERE section', () => {
  let box: HTMLElement | null = null;
  afterEach(() => { if (box) render(null, box); box?.remove(); box = null; });

  const show = (d: Partial<CardDetail>): HTMLElement => {
    box = document.createElement('div');
    document.body.append(box);
    render(h(Where, { d: { started_at: 1, ...d } as CardDetail }), box);
    return box;
  };

  it('shows the line and a strip under STARTED for a session worked on over several days', async () => {
    const detail = await fixtureSource().card('f3a379f4-1054-4974-ae7f-1b1651fc9aaa');
    expect(detail.worked).toBeTruthy();
    const el = show(detail);
    const labels = [...el.querySelectorAll('dt')].map((x) => x.textContent);
    expect(labels.indexOf('WORKED ON')).toBe(labels.indexOf('STARTED') + 1);
    expect(el.textContent).toMatch(/3 days over 4 · 4 h 43 m active · last today/);
    expect(el.querySelectorAll('.wcell').length).toBe(4);
    expect(el.querySelectorAll('.wcell.l0').length).toBe(1);
  });

  it('puts the last day of the strip on the day of the last message', async () => {
    const detail = await fixtureSource().card('f3a379f4-1054-4974-ae7f-1b1651fc9aaa');
    expect(detail.worked!.last_day).toBe(iso(detail.worked!.last_at));
  });

  it('shows one line and no strip for a session worked on one day', async () => {
    const el = show(await fixtureSource().card('e97cde56-38d0-4193-a707-980322069975'));
    expect(el.textContent).toContain('today · 25 m active');
    expect(el.querySelector('.wstrip')).toBeNull();
  });

  it('leaves the row out when the days are not counted yet', async () => {
    const detail = await fixtureSource().card('089be68b-0138-4bbc-a629-1f1259ea3a1e');
    expect(detail.worked).toBeUndefined();
    expect(show(detail).textContent).not.toContain('WORKED ON');
    expect(show({ worked: null }).textContent).not.toContain('WORKED ON');
  });

  it('says how many earlier days a long span leaves out', () => {
    const el = show({ worked: worked({ more_days: 41, span_days: 101 }) });
    expect(el.querySelector('.wstrip small')?.textContent).toBe('+41 earlier days');
  });
});
