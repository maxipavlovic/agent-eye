// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_LOOK, LOOK_LIST, lookNamed, looks, pickLook } from './index';

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

  it('takes a known look from the address for that page only: it saves nothing, here or in the engine', () => {
    const post = vi.spyOn(globalThis, 'fetch');
    expect(pickLook('?look=iris')).toBe('iris');
    expect(localStorage.getItem('eye.look')).toBeNull();
    expect(pickLook('')).toBe('ink');
    // over a saved look too
    localStorage.setItem('eye.look', 'iris');
    expect(pickLook('?look=ink')).toBe('ink');
    expect(pickLook('')).toBe('iris');
    expect(post).not.toHaveBeenCalled();
    post.mockRestore();
  });

  it('never takes a name every object has (constructor, toString, __proto__)', () => {
    for (const name of ['constructor', 'toString', '__proto__', 'hasOwnProperty']) {
      expect(pickLook(`?look=${name}`)).toBe('ink');
      localStorage.setItem('eye.look', name);
      expect(pickLook('')).toBe('ink');
      expect(lookNamed(name)).toBeNull();
    }
  });

  it('lists in the picker exactly the looks it can load', () => {
    expect(LOOK_LIST.map((l) => l.id)).toEqual(Object.keys(looks));
    expect(LOOK_LIST.map((l) => l.label)).toEqual(['INK', 'IRIS']);
    expect(lookNamed('iris')).toBe('iris');
  });
});

