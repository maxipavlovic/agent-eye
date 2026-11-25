import type { ComponentType } from 'preact';
import type { Store } from '../store';
import { DEFAULT_LOOK, lookNamed, type LookName } from './names';

export { DEFAULT_LOOK, LOOK_LIST, lookNamed, type LookName } from './names';

export type LookRoot = ComponentType<{ store: Store }>;

/** Lazy loaders, by look name: a look's CSS comes with it, so only the chosen look is fetched. */
export const looks: Record<LookName, () => Promise<LookRoot>> = {
  ink: async () => {
    await import('./ink/ink.css');
    return (await import('./ink/Root')).Root;
  },
  iris: async () => {
    await import('./iris/iris.css');
    return (await import('./iris/Root')).Root;
  },
};

const KEY = 'eye.look';

/**
 * `?look=<name>`, then the saved `eye.look` (the engine's copy is pulled into storage first), then the default.
 * A `?look=` choice holds for that page only (and its reloads, which keep the address); it saves nothing, so a
 * link with it changes no other page (the header's picker does).
 */
export function pickLook(search: string = location.search): LookName {
  const asked = lookNamed(new URLSearchParams(search).get('look'));
  if (asked) return asked;
  try {
    const saved = lookNamed(localStorage.getItem(KEY));
    if (saved) return saved;
  } catch { /* storage blocked */ }
  return DEFAULT_LOOK;
}
