/**
 * The looks by name, as the header's picker lists them. No loaders here: a look that imports this list (the
 * picker) must not pull another look's code and CSS into its bundle, and the offline single-file board would.
 */
export const LOOK_LIST = [
  { id: 'ink', label: 'INK', title: 'Ink: pixel art on warm paper' },
  { id: 'iris', label: 'IRIS', title: 'Iris: a technical drawing in one-pixel lines' },
] as const;

export type LookName = (typeof LOOK_LIST)[number]['id'];

export const DEFAULT_LOOK: LookName = 'ink';

/**
 * The list's own name matching `name`, or null: `constructor` or `toString` are not looks. It hands back the
 * list's string, never the one passed in, so a value from an address or storage is never passed on.
 */
export function lookNamed(name: unknown): LookName | null {
  return LOOK_LIST.find((l) => l.id === name)?.id ?? null;
}
