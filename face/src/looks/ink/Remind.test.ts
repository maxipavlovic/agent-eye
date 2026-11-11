// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import type { Card } from '../../api/types';
import type { Store } from '../../store';
import { remindRows } from './CardMenu';
import { dateValue } from './DatePick';
import { remindSummary, whenProblem, whenText } from './Remind';

const store = () => ({ remind: vi.fn(async () => true) }) as unknown as Store & { remind: ReturnType<typeof vi.fn> };
const card = (o: Partial<Card> = {}) => ({ id: 'a', kind: 'interactive', ...o }) as Card;
const labels = (c: Card, s: Store) => remindRows(c, s, (fn) => fn).map((r) => ('label' in r ? r.label : 'head' in r ? r.head : '-'));

describe('the REMIND ME… menu', () => {
  it('offers the quick choices and a date, and no clearing when nothing is set', () => {
    expect(labels(card(), store())).toEqual(['REMIND ME…', 'TOMORROW 9:00', 'MONDAY 9:00', 'IN A WEEK', 'PICK A DATE…']);
  });

  it('offers CLEAR REMINDER when one is set, on an idea card too', () => {
    const idea = card({ id: 'idea:2', kind: 'idea', remind_at: Date.now() + 1e6 });
    expect(labels(idea, store()).at(-1)).toBe('CLEAR REMINDER');
  });

  it('sends the words of the choice, or the clear', () => {
    const s = store();
    const rows = remindRows(card({ remind_at: 5 }), s, (fn) => fn);
    const run = (label: string) => { const r = rows.find((x) => 'label' in x && x.label === label); if (r && 'run' in r) r.run(); };
    run('TOMORROW 9:00');
    run('MONDAY 9:00');
    run('IN A WEEK');
    run('CLEAR REMINDER');
    expect(s.remind.mock.calls).toEqual([
      ['a', { when: 'tomorrow' }], ['a', { when: 'mon' }], ['a', { when: '1w' }], ['a', { clear: true }],
    ]);
  });
});

describe('the reminder form and panel', () => {
  it('sends a date and a time as one text, 9:00 when the time is empty', () => {
    expect(whenText('2026-11-20', '14:30')).toBe('2026-11-20 14:30');
    expect(whenText('2026-11-20', '')).toBe('2026-11-20 09:00');
  });

  it('writes a date as the engine reads it', () => {
    expect(dateValue(new Date(2026, 0, 5))).toBe('2026-01-05');
  });

  it('refuses a time that does not read or has passed, and takes an empty one as 9:00', () => {
    const now = new Date(2026, 10, 12, 10).getTime();
    expect(whenProblem('2026-11-13', '14:30', now)).toBeNull();
    expect(whenProblem('2026-11-13', '', now)).toBeNull();
    expect(whenProblem('2026-11-12', '930', now)).toBe('That time has passed. Pick a later one.');
    expect(whenProblem('2026-11-12', '10:15', now)).toBeNull();
    expect(whenProblem('2026-11-13', '25:00', now)).toBe('Write the time as HH:MM, 24 h, e.g. 14:30.');
  });

  it('summarises a reminder in plain words: when and the note, and Due once it is due', () => {
    const now = new Date(2026, 10, 12, 8).getTime();
    expect(remindSummary(card(), now)).toBeNull();
    expect(remindSummary(card({ remind_at: new Date(2026, 10, 13, 9).getTime(), remind_note: "Ana's reply" }), now)).toBe("Tomorrow 9:00 · Ana's reply");
    expect(remindSummary(card({ remind_at: new Date(2026, 11, 2, 14, 30).getTime() }), now)).toBe('2 Dec 14:30');
    expect(remindSummary(card({ remind_at: new Date(2026, 10, 12, 7).getTime(), remind_due: true, remind_note: 'Ana' }), now)).toBe('Due today 7:00 · Ana');
  });
});
