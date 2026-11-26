import type { Card } from '../api/types';

/** The offer the engine adds to its hint; the menu and the panel have their own button for it. */
const OFFER = /\s*Hand this to the night shift\? \(T\)$/;

/**
 * Why Continue tonight comes first on this card, in one plain line, or null: the hint applies to it (hintFor, as
 * the card you are working in) and it can be continued tonight.
 */
export function tonightWhy(c: Pick<Card, 'id' | 'kind' | 'column' | 'live_status' | 'resumable'>, hint: string | null, night: boolean): string | null {
  if (!night || !c.resumable) return null;
  const h = hintFor(c, c.id, hint);
  return h ? h.replace(OFFER, '') : null;
}

/**
 * The hint for a card, or null: only the card you have open, and only when it is a live session of yours waiting
 * on you (the one you are steering right now), while the clock says your corrections usually rise.
 */
export function hintFor(c: Pick<Card, 'id' | 'kind' | 'column' | 'live_status'>, selected: string | null, hint: string | null): string | null {
  if (!hint || c.id !== selected) return null;
  return c.kind === 'interactive' && c.live_status != null && c.column === 'needs_you' ? hint : null;
}
