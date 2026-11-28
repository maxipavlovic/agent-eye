/**
 * Board settings (theme, accent, look, layout, folded lanes) live in the browser's storage for speed, and are
 * mirrored to the engine (~/.agent-eye/ui.json) so the tab and the installed app share them and
 * they survive cleared site data.
 */

import { apiFetch, token } from './data/auth';

/** Settings every look shares. */
const SHARED = ['eye.theme', 'eye.accent', 'eye.look'] as const;

/** A setting a look keeps for itself: `eye.<look>.layout`, `.fold` or `.panel` (the engine accepts the same). */
const PER_LOOK = /^eye\.[a-z][a-z0-9]{0,15}\.(layout|fold|panel)$/;

export type PrefKey = (typeof SHARED)[number] | `eye.${string}.${'layout' | 'fold' | 'panel'}`;

export function isPrefKey(k: string): k is PrefKey {
  return (SHARED as readonly string[]).includes(k) || PER_LOOK.test(k);
}

let engine = true;

/** For builds without an engine (the offline sample board): keep settings in the browser only. */
export function keepPrefsLocal(): void {
  engine = false;
}

/** Copy the engine's saved settings into this browser's storage; quietly does nothing offline. */
export async function pullPrefs(timeoutMs = 800): Promise<void> {
  // Without the token the engine has nothing to give; the board then says how to get in
  if (!token()) return;
  try {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), timeoutMs);
    const res = await apiFetch('/api/prefs', { signal: ctl.signal });
    clearTimeout(t);
    if (!res.ok) return;
    const saved: unknown = await res.json();
    if (!saved || typeof saved !== 'object') return;
    for (const [k, v] of Object.entries(saved as Record<string, unknown>)) {
      if (isPrefKey(k) && typeof v === 'string') localStorage.setItem(k, v);
    }
  } catch { /* offline build, old engine or blocked storage */ }
}

/**
 * Remember one setting here and in the engine. Resolves true when the engine has it or there is no engine to
 * ask (the offline board, an engine that is down: the browser's copy then stands); false when the engine
 * answered with a refusal, so the caller can say so.
 */
export async function savePref(key: PrefKey, value: string, timeoutMs = 3000): Promise<boolean> {
  try { localStorage.setItem(key, value); } catch { /* storage blocked */ }
  if (!engine || !token()) return true;
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await apiFetch('/api/prefs', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ [key]: value }),
      // a page that goes away right after (a reload, a link) must not cancel the save
      keepalive: true,
      signal: ctl.signal,
    });
    // read the answer to its end: a page that reloads with the body half read has the request reported as cancelled
    await res.text();
    return res.ok;
  } catch {
    return true;
  } finally {
    clearTimeout(t);
  }
}

/** Remember one setting here and in the engine, without waiting. */
export function pushPref(key: PrefKey, value: string): void {
  void savePref(key, value);
}
