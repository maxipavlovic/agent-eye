// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from 'vitest';
import { DEFAULT_LOOK, pickLook } from './index';

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
});
