import type { Card, ClockHint } from '../api/types';

/**
 * Why Continue tonight comes first on this card, in one plain line, or null: the hint applies to it (hintFor, as
 * the card you are working in), the engine offers the night shift and the card can be continued tonight.
 */
export function tonightWhy(c: Pick<Card, 'id' | 'kind' | 'column' | 'live_status' | 'resumable'>, hint: ClockHint | null): string | null {
  if (!hint?.offer || !c.resumable) return null;
  return hintFor(c, c.id, hint.text);
}

/**
 * The hint for a card, or null: only the card you have open, and only when it is a live session of yours waiting
 * on you (the one you are steering right now), while the clock says your corrections usually rise.
 */
export function hintFor(c: Pick<Card, 'id' | 'kind' | 'column' | 'live_status'>, selected: string | null, hint: string | null): string | null {
  if (!hint || c.id !== selected) return null;
  return c.kind === 'interactive' && c.live_status != null && c.column === 'needs_you' ? hint : null;
}
