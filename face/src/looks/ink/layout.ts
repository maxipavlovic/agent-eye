import { useSignal } from '@preact/signals';
import { pushPref } from '../../prefs';
import type { ColumnId } from '../../api/types';
import { FOLDABLE } from './columns';

export type Layout = 'lanes' | 'grid';
export type SetFold = (id: ColumnId, folded: boolean) => void;

function load(k: string, d: string): string {
  try { return localStorage.getItem(k) ?? d; } catch { return d; }
}

/** Saved folded lanes (a JSON list); without one, Done is folded unless the old Done switch said open. */
export function foldedFrom(saved: string, oldDone: string): ColumnId[] {
  try {
    const v: unknown = JSON.parse(saved);
    if (Array.isArray(v)) return v.filter((x): x is ColumnId => typeof x === 'string' && FOLDABLE.has(x as ColumnId));
  } catch { /* nothing saved yet */ }
  return oldDone === 'open' ? [] : ['done'];
}

/** The board layout and its folded lanes, remembered in the browser and in the engine. */
export function useLayout() {
  const layout = useSignal<Layout>(load('eye.ink.layout', 'lanes') === 'grid' ? 'grid' : 'lanes');
  // Folded lanes; Done starts folded. An older saved "Done open" choice carries over once.
  const folded = useSignal<ReadonlySet<ColumnId>>(new Set(foldedFrom(load('eye.ink.fold', ''), load('eye.ink.done', 'folded'))));
  const setLayout = (l: Layout) => { layout.value = l; pushPref('eye.ink.layout', l); };
  const setFold: SetFold = (id, v) => {
    const next = new Set(folded.value);
    if (v) next.add(id); else next.delete(id);
    folded.value = next;
    pushPref('eye.ink.fold', JSON.stringify([...next]));
  };
  return { layout: layout.value, folded: folded.value, setLayout, setFold };
}
