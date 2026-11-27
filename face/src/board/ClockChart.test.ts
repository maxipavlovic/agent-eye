import { describe, expect, it } from 'vitest';
import { demoClock } from '../data/clockSample';
import { bandNames } from './ClockChart';

describe('the names of the bad stretches', () => {
  it('stay on one row when there is room, and a name that would reach the edge ends at its band', () => {
    const one = bandNames({ ...demoClock(4), bad: [{ from_h: 9, to_h: 12, label: 'morning' }] }, false);
    expect(one).toEqual([expect.objectContaining({ text: 'your morning', row: 0, right: false })]);
    const late = bandNames({ ...demoClock(4), bad: [{ from_h: 21, to_h: 23, label: 'evening' }] }, false);
    expect(late[0]).toMatchObject({ right: true, row: 0 });
  });

  it('put a name that would touch the one before it on the next row, on a phone', () => {
    const two = bandNames({ ...demoClock(4), bad: [{ from_h: 6, to_h: 8, label: 'morning' }, { from_h: 9, to_h: 11, label: 'afternoon' }] }, true);
    expect(two.map((n) => n.row)).toEqual([0, 1]);
  });
});
