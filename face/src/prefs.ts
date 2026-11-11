/**
 * Board settings (theme, accent, layout, folded lanes) live in the browser's storage for speed, and are
 * mirrored to the engine (~/.agent-eye/ui.json) so the tab and the installed app share them and
 * they survive cleared site data.
 */
export const PREF_KEYS = ['eye.theme', 'eye.ink.layout', 'eye.ink.fold', 'eye.accent'] as const;

let engine = true;

/** For builds without an engine (the offline sample board): keep settings in the browser only. */
export function keepPrefsLocal(): void {
  engine = false;
}

/** Copy the engine's saved settings into this browser's storage; quietly does nothing offline. */
export async function pullPrefs(timeoutMs = 800): Promise<void> {
  try {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), timeoutMs);
    const res = await fetch('/api/prefs', { credentials: 'same-origin', signal: ctl.signal });
    clearTimeout(t);
    if (!res.ok) return;
    const saved: unknown = await res.json();
    if (!saved || typeof saved !== 'object') return;
    for (const k of PREF_KEYS) {
      const v = (saved as Record<string, unknown>)[k];
      if (typeof v === 'string') localStorage.setItem(k, v);
    }
  } catch { /* offline build, old engine or blocked storage */ }
}

/** Remember one setting here and in the engine. */
export function pushPref(key: (typeof PREF_KEYS)[number], value: string): void {
  try { localStorage.setItem(key, value); } catch { /* storage blocked */ }
  if (!engine) return;
  void fetch('/api/prefs', {
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ [key]: value }),
  }).catch(() => { /* offline build */ });
}
