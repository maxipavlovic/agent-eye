import type { ComponentChildren } from 'preact';
import { useSignal } from '@preact/signals';

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

/** The categorical slots in a fixed order (validated light and dark in `charts.css`); "other" is always the neutral one. */
export const SLOTS = 5;

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

/** The colour token of a slot. */
export const slotVar = (s: number | 'x') => `var(--c${s === 'x' ? 'x' : s + 1})`;

/** Resets the memory of key colours (tests). */
export function forgetSlots(): void {
  slots.clear();
}

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

/** A chart's frame: the title row with the chart/table switch, and the tooltip over the drawing. */
export function ChartFrame({ title, table, onTable, hint, children }: { title: string; table: boolean; onTable: (t: boolean) => void; hint?: string; children: ComponentChildren }) {
  return (
    <div class="chart">
      <div class="chead">
        <span class="ctitle">{title}</span>
        <button type="button" class="ctoggle" aria-pressed={table} onClick={() => onTable(!table)}>{table ? 'CHART' : 'TABLE'}</button>
      </div>
      {hint && <p class="chint">{hint}</p>}
      {children}
    </div>
  );
}
