// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_LOOK, isLook, LOOK_LIST, looks, pickLook } from './index';

describe('pickLook', () => {
  beforeEach(() => localStorage.clear());

  it('defaults to ink', () => {
    expect(pickLook('')).toBe('ink');
    expect(DEFAULT_LOOK).toBe('ink');
  });

  it('falls back to ink for an unknown look, asked or saved', () => {
    expect(pickLook('?look=unknown')).toBe('ink');
    localStorage.setItem('eye.look', 'old');
    expect(pickLook('')).toBe('ink');
  });

  it('never takes a name every object has (constructor, toString, __proto__)', () => {
    for (const name of ['constructor', 'toString', '__proto__', 'hasOwnProperty']) {
      expect(pickLook(`?look=${name}`)).toBe('ink');
      localStorage.setItem('eye.look', name);
      expect(pickLook('')).toBe('ink');
      expect(isLook(name)).toBe(false);
    }
  });

  it('lists in the picker exactly the looks it can load', () => {
    expect(LOOK_LIST.map((l) => l.id)).toEqual(Object.keys(looks));
    for (const l of LOOK_LIST) expect(l.label).toBe(l.id.toUpperCase());
  });
});

