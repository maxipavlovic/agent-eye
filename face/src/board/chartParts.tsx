import type { ComponentChildren } from 'preact';
import { signal, useSignal } from '@preact/signals';
import { SLOTS } from './chartPalette';

// What every chart shares, in both looks: the column path, the tooltip and the colours. The looks only set the
// tokens in `charts.css`; no chart library, inline SVG like the rest of the face.

/** A column from `base` to `end` (y grows downward), 4px rounded at the data end and square at the baseline. */
export function colPath(x: number, w: number, base: number, end: number, r = 4): string {
  const h = Math.abs(end - base);
  if (h < 0.5) return '';
  const k = Math.min(r, h, w / 2);
  const up = end < base;
  const x1 = x + w;
  // the rounded end first, then straight down (or up) to the baseline
  return up
    ? `M${x},${base}V${end + k}Q${x},${end} ${x + k},${end}H${x1 - k}Q${x1},${end} ${x1},${end + k}V${base}Z`
    : `M${x},${base}V${end - k}Q${x},${end} ${x + k},${end}H${x1 - k}Q${x1},${end} ${x1},${end - k}V${base}Z`;
}

// The categorical slots come from `chartPalette.ts`; "other" is always the neutral one.

/** A key keeps its colour for as long as the page lives: it takes the lowest slot the keys on screen do not use. */
const slots = new Map<string, number>();

export function slotOf(key: string, onScreen: readonly string[]): number | 'x' {
  if (key === 'other') return 'x';
  let s = slots.get(key);
  if (s === undefined || onScreen.some((k) => k !== key && slots.get(k) === s)) {
    const used = new Set(onScreen.filter((k) => k !== key && k !== 'other').map((k) => slots.get(k)));
    s = Array.from({ length: SLOTS }, (_, i) => i).find((i) => !used.has(i)) ?? 0;
    slots.set(key, s);
  }
  return s;
}

/** The colour of a slot among the `colours` this accent leaves (`liveColours`); "other" is the neutral token. */
export const slotColour = (s: number | 'x', colours: readonly string[]) => (s === 'x' ? 'var(--cx)' : colours[s] ?? 'var(--cx)');

/** Resets the memory of key colours (tests). */
export function forgetSlots(): void {
  slots.clear();
}

/** Is the screen phone-narrow (the media query the charts' own CSS uses)? The charts draw a narrower picture then, so their words stay readable. */
const narrow = signal(false);
if (typeof matchMedia === 'function') {
  const q = matchMedia('(max-width: 640px)');
  narrow.value = q.matches;
  q.addEventListener('change', (e) => { narrow.value = e.matches; });
}
export const useNarrow = () => narrow.value;

export interface TipState { left: number; lines: Array<{ key?: string; text: string; strong?: string }>; title: string }

/** One tooltip for a chart: where it sits (a share of the chart's width) and what it says. Shown on hover and on focus. */
export function useTip() {
  const tip = useSignal<TipState | null>(null);
  return { tip: tip.value, show: (t: TipState) => { tip.value = t; }, hide: () => { tip.value = null; } };
}

/** The tooltip box. Texts go in as text nodes: names come from the data. */
export function Tip({ tip }: { tip: TipState | null }) {
  if (!tip) return null;
  const right = tip.left > 55;
  return (
    <div class="ctip" role="status" style={right ? `right:${100 - tip.left}%` : `left:${tip.left}%`}>
      <b>{tip.title}</b>
      {tip.lines.map((l, i) => (
        <p key={i}>{l.key && <i class="ckey" style={`background:${l.key}`} />}{l.strong && <strong>{l.strong}</strong>}{l.text}</p>
      ))}
    </div>
  );
}

/** A chart's frame: the title row with the chart/table switch (when the chart has a table), and the drawing. */
export function ChartFrame({ title, table = false, onTable, hint, children }: { title: string; table?: boolean; onTable?: (t: boolean) => void; hint?: string; children: ComponentChildren }) {
  return (
    <div class="chart">
      <div class="chead">
        <span class="ctitle">{title}</span>
        {onTable && <button type="button" class="ctoggle" aria-pressed={table} onClick={() => onTable(!table)}>{table ? 'CHART' : 'TABLE'}</button>}
      </div>
      {hint && <p class="chint">{hint}</p>}
      {children}
    </div>
  );
}
