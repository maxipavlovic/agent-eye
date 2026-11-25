import type { ComponentType } from 'preact';
import type { Store } from '../store';

export type LookRoot = ComponentType<{ store: Store }>;

/** Every look: how the header's picker names it, and how it loads (its CSS comes with it, so only the chosen look is fetched). */
const REGISTRY = {
  ink: {
    label: 'INK',
    title: 'Ink: pixel art on warm paper',
    load: async (): Promise<LookRoot> => {
      await import('./ink/ink.css');
      return (await import('./ink/Root')).Root;
    },
  },
  iris: {
    label: 'IRIS',
    title: 'Iris: a technical drawing in one-pixel lines',
    load: async (): Promise<LookRoot> => {
      await import('./iris/iris.css');
      return (await import('./iris/Root')).Root;
    },
  },
} as const;

export type LookName = keyof typeof REGISTRY;

/**
 * The registry's own name matching `name`, or null: `constructor` or `toString` are not looks. It hands back the
 * registry's string, never the one passed in, so what came from an address never reaches storage.
 */
export function lookNamed(name: unknown): LookName | null {
  return (Object.keys(REGISTRY) as LookName[]).find((id) => id === name) ?? null;
}

/** Lazy loaders, by look name. */
export const looks: Record<LookName, () => Promise<LookRoot>> = Object.fromEntries(
  Object.entries(REGISTRY).map(([id, l]) => [id, l.load]),
) as Record<LookName, () => Promise<LookRoot>>;

/** The looks as the header's picker lists them. */
export const LOOK_LIST: ReadonlyArray<{ id: LookName; label: string; title: string }> = Object.entries(REGISTRY).map(
  ([id, l]) => ({ id: id as LookName, label: l.label, title: l.title }),
);

export const DEFAULT_LOOK: LookName = 'ink';
const KEY = 'eye.look';

/**
 * `?look=<name>`, then the saved `eye.look` (the engine's copy is pulled into storage first), then the default.
 * A `?look=` choice is remembered in this browser only: a link with it does not change the look of the others
 * (the header's picker does).
 */
export function pickLook(search: string = location.search): LookName {
  const asked = lookNamed(new URLSearchParams(search).get('look'));
  if (asked) {
    try { localStorage.setItem(KEY, asked); } catch { /* storage blocked */ }
    return asked;
  }
  try {
    const saved = lookNamed(localStorage.getItem(KEY));
    if (saved) return saved;
  } catch { /* storage blocked */ }
  return DEFAULT_LOOK;
}
