import { describe, expect, it } from 'vitest';
import type { Card } from '../api/types';
import { cardMarks, marksWidth, MAX_MARKS, shownMarks } from './Card';

const card = (over: Partial<Card>): Card => ({ id: 's', kind: 'interactive', title: 't', ...over }) as Card;
const full = card({ remind_at: Date.now() + 3_600_000, night: { id: 1, status: 'done' }, loose: 3, remote_url: 'https://x' });

describe('the marks on a card\'s top edge', () => {
  it('are the card\'s states, most important first: Needs you, a reminder, a night run, leftovers, Remote Control', () => {
    expect(cardMarks(full, true).map((m) => m.kind)).toEqual(['need', 'clock', 'night', 'loose', 'rc']);
    expect(cardMarks(full, false).map((m) => m.kind)).toEqual(['clock', 'night', 'loose', 'rc']);
    expect(cardMarks(card({}), false)).toEqual([]);
  });

  it('carry the clock\'s hint as one more calm mark, after Needs you, when the card is given one', () => {
    expect(cardMarks(full, true, 'Your corrections usually rise now').map((m) => m.kind)).toEqual(['need', 'hours', 'clock', 'night', 'loose', 'rc']);
    expect(cardMarks(card({}), false, 'Your corrections usually rise now')).toEqual([{ kind: 'hours', title: 'Your corrections usually rise now' }]);
  });

  it('an idea shows only Needs you and its reminder', () => {
    expect(cardMarks({ ...full, kind: 'idea' } as Card, true).map((m) => m.kind)).toEqual(['need', 'clock']);
  });

  it('say what they mean in words, for the tooltip and a screen reader', () => {
    const t = Object.fromEntries(cardMarks(full, true).map((m) => [m.kind, m.title]));
    expect(t.loose).toBe('3 leftovers: open the card to see what the session left behind');
    expect(t.rc).toBe('Remote Control is on for this session');
    expect(cardMarks(card({ loose: 1 }), false)[0]!.title).toMatch(/^1 leftover: /);
  });

  it(`never take more than ${MAX_MARKS} boxes: the first ones and +N for the rest`, () => {
    const all = cardMarks(full, true);
    expect(shownMarks(all.slice(0, 4))).toEqual({ shown: all.slice(0, 4), more: 0 });
    expect(shownMarks(all)).toEqual({ shown: all.slice(0, 3), more: 2 });
  });

  it('tell the top line how much room they take, a two-character box counted wider', () => {
    expect(marksWidth(full, [])).toBe(0);
    expect(marksWidth(full, cardMarks(card({ remind_at: Date.now() + 1 }), true))).toBe(60);
    expect(marksWidth(card({ loose: 12 }), cardMarks(card({ loose: 12 }), false))).toBe(40);
    // 3 boxes and a +2 box
    expect(marksWidth(full, cardMarks(full, true))).toBe(3 * 30 + 40);
  });
});
