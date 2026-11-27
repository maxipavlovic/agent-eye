import { useSignal } from '@preact/signals';
import type { UsageSeries } from '../api/types';
import { colPath, slotColour, slotOf, Tip, useNarrow, useTip } from './chartParts';
import { liveColours } from './chartPalette';
import { costMax, dayCost, dayLabel, dayTotal, dollars, keyCostText } from './dailyCost';

// The daily cost of the window as stacked columns, by project or by model. A key keeps its colour whatever its rank.
// Like HOURS, it is one slider (arrow keys read each day), and on a phone the drawing is 320 wide, so its words keep
// their size when it shrinks to the screen.

const TOP = 12;
const BOTTOM = 190;
const GAP = 2;

export function UsageDaysChart({ series }: { series: UsageSeries }) {
  const narrow = useNarrow();
  const W = narrow ? 320 : 720;
  const X0 = narrow ? 44 : 52;
  const { tip, show, hide } = useTip();
  const active = useSignal<number | null>(null);
  const max = costMax(series);
  const n = series.days.length;
  const slot = (W - X0 - 8) / n;
  const bw = Math.min(24, slot - GAP * 2);
  const colours = liveColours();
  const colour = (k: string) => slotColour(slotOf(k, series.keys), colours);
  const y = (v: number) => BOTTOM - (v / max) * (BOTTOM - TOP);
  const tipOf = (i: number) => {
    const d = series.days[i]!;
    active.value = i;
    show({
      left: ((X0 + slot * i + slot / 2) / W) * 100,
      title: `${dayLabel(d.day)} · ${dollars(dayTotal(d))}`,
      lines: series.keys.flatMap((k) => ((d.tokens[k] ?? 0) > 0 || (d.cost[k] ?? 0) > 0 ? [{ key: colour(k), text: ` ${k}`, strong: keyCostText(d, k) }] : [])),
    });
  };
  const off = () => { active.value = null; hide(); };
  const dayAt = (e: PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    return Math.max(0, Math.min(n - 1, Math.floor(((e.clientX - r.left) / r.width * W - X0) / slot)));
  };
  const onKey = (e: KeyboardEvent) => {
    const at = active.value ?? -1;
    const next = e.key === 'ArrowRight' ? Math.min(n - 1, at + 1) : e.key === 'ArrowLeft' ? Math.max(0, at < 0 ? 0 : at - 1) : e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : null;
    if (next === null) return;
    e.preventDefault();
    tipOf(next);
  };
  // a label under every column of a short window, every few of a long one
  const every = n > 14 ? Math.ceil(n / (narrow ? 5 : 7)) : 1;
  return (
    <div class="cwrap cfocus" role="slider" tabIndex={0} aria-label={`Cost by day, by ${series.by}. Use the arrow keys to read each day.`} aria-orientation="horizontal" aria-valuemin={0} aria-valuemax={n - 1} aria-valuenow={active.value ?? 0} aria-valuetext={active.value === null ? undefined : `${dayLabel(series.days[active.value]!.day)}: ${dollars(dayTotal(series.days[active.value]!))}`} onFocus={() => { if (active.value === null && n > 0) tipOf(0); }} onBlur={off} onKeyDown={onKey}>
      <svg class="cdiagram" viewBox={`0 0 ${W} 236`} aria-hidden="true" onPointerMove={(e) => tipOf(dayAt(e))} onPointerDown={(e) => tipOf(dayAt(e))} onPointerLeave={(e) => { if (document.activeElement !== e.currentTarget.parentElement) off(); }}>
        <line class="cgrid" x1={X0} x2={W - 8} y1={TOP} y2={TOP} />
        <line class="cgrid" x1={X0} x2={W - 8} y1={(TOP + BOTTOM) / 2} y2={(TOP + BOTTOM) / 2} />
        <text class="clab" x={X0 - 8} y={TOP + 4} text-anchor="end">{dollars(max, true)}</text>
        <text class="clab" x={X0 - 8} y={(TOP + BOTTOM) / 2 + 4} text-anchor="end">{dollars(max / 2, true)}</text>
        <text class="clab" x={X0 - 8} y={BOTTOM + 4} text-anchor="end">$0</text>
        {active.value !== null && <rect class="cact" x={X0 + slot * active.value} width={slot} y={0} height={BOTTOM + 1} />}
        {series.days.map((d, i) => {
          const x = X0 + slot * i + (slot - bw) / 2;
          let acc = 0;
          const segs = series.keys.flatMap((k) => {
            const c = dayCost(d, k);
            if (!c) return [];
            const from = y(acc);
            acc += c;
            return [{ k, from, to: y(acc) }];
          });
          return (
            <g key={d.day}>
              {segs.map((s, j) => {
                const top = j === segs.length - 1;
                // a 2px surface gap between segments: each one stops short of the one below it
                const base = s.from - (j === 0 ? 0 : GAP);
                return top ? <path key={s.k} d={colPath(x, bw, base, s.to)} style={`fill:${colour(s.k)}`} /> : <rect key={s.k} x={x} width={bw} y={s.to} height={Math.max(0, base - s.to)} style={`fill:${colour(s.k)}`} />;
              })}
              {i % every === 0 && <text class="clab" x={X0 + slot * i + slot / 2} y={BOTTOM + 20} text-anchor="middle">{d.day.slice(8)}</text>}
            </g>
          );
        })}
        <line class="cbase" x1={X0} x2={W - 8} y1={BOTTOM} y2={BOTTOM} />
        <text class="clab" x={W - 8} y={BOTTOM + 42} text-anchor="end">day of the month</text>
      </svg>
      <Tip tip={tip} />
    </div>
  );
}

/** The legend: a swatch and the key in text colour, always shown for two or more keys. */
export function UsageLegend({ series }: { series: UsageSeries }) {
  if (series.keys.length < 2) return null;
  const colours = liveColours();
  return (
    <ul class="clegend" aria-label="Colours">
      {series.keys.map((k) => <li key={k}><i class="ckey sw" style={`background:${slotColour(slotOf(k, series.keys), colours)}`} /><span title={k}>{k}</span></li>)}
    </ul>
  );
}

/** The numbers behind the chart: a row per day, a column per key and the total. */
export function UsageDaysTable({ series }: { series: UsageSeries }) {
  return (
    <table class="ctable">
      <caption>Cost by day, by {series.by}</caption>
      <thead><tr><th>Day</th>{series.keys.map((k) => <th key={k}>{k}</th>)}<th>Total</th></tr></thead>
      <tbody>
        {series.days.map((d) => (
          <tr key={d.day}>
            <th scope="row">{d.day}</th>
            {series.keys.map((k) => <td key={k}>{(d.tokens[k] ?? 0) > 0 || (d.cost[k] ?? 0) > 0 ? keyCostText(d, k) : '–'}</td>)}
            <td>{dollars(dayTotal(d))}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
