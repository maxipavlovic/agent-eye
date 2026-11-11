/** The accent colours of the board: the ink look's signal colour, chosen in the header. The colours
 *  themselves are in `looks/ink/tokens.css`, one `data-accent` block per id. */
export const ACCENTS = [
  { id: 'vermilion', name: 'Vermilion (original)' },
  { id: 'cobalt', name: 'Cobalt' },
  { id: 'teal', name: 'Teal' },
  { id: 'moss', name: 'Moss' },
  { id: 'amber', name: 'Amber' },
  { id: 'magenta', name: 'Magenta' },
  { id: 'violet', name: 'Violet' },
  { id: 'graphite', name: 'Graphite' },
] as const;

export type AccentId = (typeof ACCENTS)[number]['id'];

export const DEFAULT_ACCENT: AccentId = 'vermilion';

/** A saved value as an accent id; anything unknown or missing is the default. */
export function parseAccent(v: unknown): AccentId {
  return ACCENTS.find((a) => a.id === v)?.id ?? DEFAULT_ACCENT;
}
