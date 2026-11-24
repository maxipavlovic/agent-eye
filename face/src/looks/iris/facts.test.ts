import { describe, expect, it } from 'vitest';
import type { Card } from '../../api/types';
import { hoursAt, laneFact, tickAt } from './facts';

const NOW = 1_795_176_000_000;
const H = 3_600_000;

const card = (over: Partial<Card>): Card => ({ id: Math.random().toString(36).slice(2), agent_name: 'Claude', last_activity_at: NOW - H, next: null, live_status: null, ...over }) as Card;

describe('the fact under a lane name', () => {
  it('Needs you: how long the oldest ask has waited', () => {
    expect(laneFact('needs_you', [card({ last_activity_at: NOW - 5 * H }), card({ last_activity_at: NOW - 30 * 60_000 })], NOW)).toBe('oldest ask 5h');
    expect(laneFact('needs_you', [card({ last_activity_at: NOW - 10_000 })], NOW)).toBe('just asked');
    expect(laneFact('needs_you', [], NOW)).toBe('nothing waits on you');
    expect(laneFact('needs_you', [card({ last_activity_at: null }), card({ last_activity_at: null })], NOW)).toBe('2 waiting on you');
  });

  it('Waiting: who, each name once, in board order; "others" when nobody is named', () => {
    const cards = [card({ next: 'Dana: review PR 412' }), card({ next: 'Sam: approve' }), card({ next: 'Dana: add the CDN rule' }), card({ next: 'Claude: rewrite' })];
    expect(laneFact('waiting', cards, NOW)).toBe('on Dana, Sam');
    expect(laneFact('waiting', [card({ next: 'Claude: rewrite the welcome screen' })], NOW)).toBe('on others');
    expect(laneFact('waiting', [], NOW)).toBe('nobody to wait for');
    const four = ['Ana', 'Bo', 'Cy', 'Di'].map((n) => card({ next: `${n}: review` }));
    expect(laneFact('waiting', four, NOW)).toBe('on Ana, Bo +2');
  });

  it('Running: how many are working right now', () => {
    expect(laneFact('running', [card({ live_status: 'busy' }), card({ live_status: 'idle' }), card({ live_status: 'busy' })], NOW)).toBe('2 working now');
    expect(laneFact('running', [card({ live_status: 'idle' })], NOW)).toBe('none working right now');
  });

  it('Paused: how many moved this week', () => {
    expect(laneFact('paused', [card({ last_activity_at: NOW - 2 * 24 * H }), card({ last_activity_at: NOW - 9 * 24 * H })], NOW)).toBe('1 this week');
    expect(laneFact('paused', [card({ last_activity_at: NOW - 9 * 24 * H })], NOW)).toBe('none this week');
  });

  it('Ideas and Done: the newest', () => {
    expect(laneFact('ideas', [card({ last_activity_at: NOW - 3 * 24 * H }), card({ last_activity_at: NOW - 26 * H })], NOW)).toBe('newest 1d');
    expect(laneFact('done', [], NOW)).toBe('nothing done yet');
    expect(laneFact('ideas', [], NOW)).toBe('no ideas yet');
    expect(laneFact('ideas', [card({ last_activity_at: null })], NOW)).toBe('1 parked');
    expect(laneFact('done', [card({ last_activity_at: null }), card({ last_activity_at: null })], NOW)).toBe('2 finished');
  });
});

describe('a tick on the timeline', () => {
  it('sits at the right for now and at the left for two weeks ago and before', () => {
    expect(tickAt(NOW, NOW)).toBe(1);
    expect(tickAt(NOW - 14 * 24 * H, NOW)).toBeCloseTo(0, 6);
    expect(tickAt(NOW - 40 * 24 * H, NOW)).toBe(0);
    expect(tickAt(null, NOW)).toBeNull();
  });

  it('puts the scale marks where ticks of that age would sit', () => {
    expect(hoursAt(0)).toBe(1);
    expect(hoursAt(336)).toBeCloseTo(0, 6);
    for (const h of [1, 24, 168]) expect(hoursAt(h)).toBe(tickAt(NOW - h * H, NOW));
  });

  it('moves left as a session gets older, the last day taking more room than last week', () => {
    const at = [0, 1, 6, 24, 72, 168, 336].map((h) => tickAt(NOW - h * H, NOW)!);
    for (let i = 1; i < at.length; i++) expect(at[i]).toBeLessThan(at[i - 1]!);
    // the first day spans more of the line than days 7 to 14
    expect(1 - at[3]!).toBeGreaterThan(at[5]! - at[6]!);
  });
});
