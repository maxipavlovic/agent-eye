import type { Card } from '../api/types';

/**
 * The hint for a card, or null: only the card you have open, and only when it is a live session of yours waiting
 * on you (the one you are steering right now), while the clock says your corrections usually rise.
 */
export function hintFor(c: Pick<Card, 'id' | 'kind' | 'column' | 'live_status'>, selected: string | null, hint: string | null): string | null {
  if (!hint || c.id !== selected) return null;
  return c.kind === 'interactive' && c.live_status != null && c.column === 'needs_you' ? hint : null;
}
