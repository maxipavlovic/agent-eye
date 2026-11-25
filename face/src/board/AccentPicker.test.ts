// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Store } from '../store';
import { switchLook } from './AccentPicker';

describe('switching the look', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.restoreAllMocks());

  it('puts the browser back on its look when the engine refuses the new one, and says so', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(() => Promise.resolve(new Response('{"error":"full"}', { status: 400 })));
    const store = { toast: vi.fn() } as unknown as Store;
    localStorage.setItem('eye.look', 'ink');
    await switchLook(store, 'iris');
    expect(localStorage.getItem('eye.look')).toBe('ink');
    // nothing saved before: nothing saved after
    localStorage.clear();
    await switchLook(store, 'iris');
    expect(localStorage.getItem('eye.look')).toBeNull();
    expect(store.toast).toHaveBeenCalledTimes(2);
  });
});
