// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { isPrefKey, pullPrefs, savePref } from './prefs';

// The same lists as the engine's test (src/server/prefs.rs): the two must agree, or a setting is refused.
const OK = ['eye.theme', 'eye.accent', 'eye.look', 'eye.ink.layout', 'eye.ink.fold', 'eye.ink.panel', 'eye.iris.layout', 'eye.iris2.fold'];
const BAD = ['eye.iris.colour', 'eye.Iris.layout', 'eye.2iris.layout', 'eye.ir-is.layout', 'eye..layout', 'eye.iris', 'eye.a.b.layout', 'eye.abcdefghijklmnopq.layout', 'iris.layout', 'eye.looks'];

const answer = (status: number, body: unknown = {}) => Promise.resolve(new Response(JSON.stringify(body), { status }));

describe('board settings', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.restoreAllMocks());

  it('knows the same keys as the engine', () => {
    for (const k of OK) expect(isPrefKey(k), k).toBe(true);
    for (const k of BAD) expect(isPrefKey(k), k).toBe(false);
  });

  it('copies only setting keys from the engine into this browser', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(() => answer(200, { 'eye.iris.layout': 'grid', 'eye.look': 'iris', 'eye.secret': 'x', 'eye.theme': 3 }));
    await pullPrefs();
    expect(localStorage.getItem('eye.iris.layout')).toBe('grid');
    expect(localStorage.getItem('eye.look')).toBe('iris');
    expect(localStorage.getItem('eye.secret')).toBeNull();
    expect(localStorage.getItem('eye.theme')).toBeNull();
  });

  it('says when the engine refuses a setting, and keeps going when there is no engine to ask', async () => {
    const f = vi.spyOn(globalThis, 'fetch').mockImplementation(() => answer(400, { error: 'unknown setting' }));
    expect(await savePref('eye.look', 'iris')).toBe(false);
    f.mockImplementation(() => answer(200));
    expect(await savePref('eye.look', 'iris')).toBe(true);
    f.mockImplementation(() => Promise.reject(new TypeError('Failed to fetch')));
    expect(await savePref('eye.look', 'ink')).toBe(true);
    // the browser's copy is written either way
    expect(localStorage.getItem('eye.look')).toBe('ink');
  });

  it('saves with keepalive, so leaving the page right after does not cancel the request', async () => {
    const f = vi.spyOn(globalThis, 'fetch').mockImplementation(() => answer(200));
    await savePref('eye.theme', 'dark');
    expect(f.mock.calls[0]![1]).toMatchObject({ method: 'POST', keepalive: true });
  });
});
