import type { ComponentType } from 'preact';
import type { Store } from '../store';

export type LookRoot = ComponentType<{ store: Store }>;

/** Lazy loaders: a look's CSS is imported by its loader, so only the chosen look is fetched. */
export const looks: Record<string, () => Promise<LookRoot>> = {
  ink: async () => {
    await import('./ink/ink.css');
    return (await import('./ink/Root')).Root;
  },
};

export const DEFAULT_LOOK = 'ink';
const KEY = 'eye.look';

/** `?look=<name>`, then localStorage `eye.look`, then the default. A `?look=` choice is remembered. */
export function pickLook(search: string = location.search): string {
  const asked = new URLSearchParams(search).get('look');
  if (asked && asked in looks) {
    try { localStorage.setItem(KEY, asked); } catch { /* storage blocked */ }
    return asked;
  }
  try {
    const saved = localStorage.getItem(KEY);
    if (saved && saved in looks) return saved;
  } catch { /* storage blocked */ }
  return DEFAULT_LOOK;
}
