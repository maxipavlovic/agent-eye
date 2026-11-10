import { useRef } from 'preact/hooks';
import type { Store } from '../../store';
import type { Column, ColumnId } from '../../api/types';

/** The panel's place in the walk: `at` is -1 when the card is not on the board (e.g. a search hit). */
export interface Walk { at: number; total: number; step: (dir: 1 | -1) => void }

/**
 * ← / → walk the cards in the order the board had when the panel opened (unfolded lanes, left to
 * right, top to bottom, current project filter), so moving a card does not reshuffle the walk.
 */
export function useWalk(store: Store, cols: Column[], folded: ReadonlySet<ColumnId>): Walk {
  const sel = store.selected.value;
  const frozen = useRef<string[] | null>(null);
  if (!sel) frozen.current = null;
  else if (!frozen.current) frozen.current = cols.filter((c) => !folded.has(c.id)).flatMap((c) => c.cards.map((x) => x.id));
  const order = frozen.current ?? [];
  const step = (dir: 1 | -1) => {
    const cur = store.selected.value;
    if (!cur || !order.length) return;
    const i = order.indexOf(cur);
    store.open(order[i < 0 ? 0 : (i + dir + order.length) % order.length]!);
  };
  return { at: sel ? order.indexOf(sel) : -1, total: order.length, step };
}
