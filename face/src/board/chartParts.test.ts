import { beforeEach, describe, expect, it } from 'vitest';
import { colPath, forgetSlots, slotOf, slotVar } from './chartParts';

describe('chart parts', () => {
  beforeEach(forgetSlots);

  it('draws a column with a 4px rounded top and a square foot', () => {
    expect(colPath(0, 20, 100, 50)).toBe('M0,100V54Q0,50 4,50H16Q20,50 20,54V100Z');
    expect(colPath(0, 20, 100, 150)).toBe('M0,100V146Q0,150 4,150H16Q20,150 20,146V100Z');
  });

  it('rounds no more than the column is tall, and draws nothing for no height', () => {
    expect(colPath(0, 20, 100, 98)).toBe('M0,100V100Q0,98 2,98H18Q20,98 20,100V100Z');
    expect(colPath(0, 20, 100, 100.2)).toBe('');
  });

  it('keeps the colour of a key whatever its rank, and gives other the neutral one', () => {
    const first = slotOf('billing', ['billing', 'docs']);
    const second = slotOf('docs', ['billing', 'docs']);
    expect(first).not.toBe(second);
    // docs now ranks first and billing is gone: nothing is repainted
    expect(slotOf('docs', ['docs', 'site'])).toBe(second);
    expect(slotOf('other', ['docs', 'other'])).toBe('x');
    expect(slotVar('x')).toBe('var(--cx)');
    expect(slotVar(0)).toBe('var(--c1)');
  });

  it('gives a new key a slot the others on screen do not use', () => {
    const on = ['a', 'b', 'c', 'd', 'e'];
    const got = on.map((k) => slotOf(k, on));
    expect(new Set(got).size).toBe(5);
    // a sixth key replaces one: it takes a slot no key still on screen holds
    const next = ['f', 'b', 'c', 'd', 'e'];
    expect(next.map((k) => slotOf(k, next)).length).toBe(5);
    expect(new Set(next.map((k) => slotOf(k, next))).size).toBe(5);
  });
});
