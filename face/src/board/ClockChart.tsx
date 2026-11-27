import { useSignal } from '@preact/signals';
import type { ClockReport, ClockBand } from '../api/types';
import { colPath, Tip, useNarrow, useTip } from './chartParts';
import {
  bandLabel, bandSpans, breakLine, clockUnit, FAINT_BELOW, hourRange, modelLine, niceMax, per100, rate1, thinkBaseline, thinkPercent, thinkSpan, toolLine, youLine,
} from './clockModel';

// The HOURS diagram: one 24-hour axis in local time, shared by three strips (you, the model, tool errors), each with
// its own unit and no second scale. All three are read by the same hover or focus: one hour, one tooltip. On a phone
// the drawing is 320 wide instead of 720, so its words keep their size when it shrinks to the screen.

const TITLE = 22;
/** The row above the strips that names the bad stretches. */
const BANDS = 22;

/** Where everything sits, for the wide drawing and the phone one. */
function layout(narrow: boolean) {
  const W = narrow ? 320 : 720;
  const X0 = narrow ? 40 : 48;
  const SLOT = (W - X0 - (narrow ? 4 : 8)) / 24;
  const strips = { you: { y: BANDS, h: 120 }, model: { y: BANDS + 160, h: 96 }, tool: { y: BANDS + 296, h: 44 } };
  return { W, X0, SLOT, BAR: Math.min(20, SLOT - (narrow ? 4 : 6)), R: W - (narrow ? 4 : 8), strips, H: BANDS + 372, hx: (h: number) => X0 + h * SLOT };
}

/** The tooltip of an hour: what you did, what the model did, what the tools did. */
function hourTip(c: ClockReport, h: number, base: number | null, g: ReturnType<typeof layout>) {
  const row = c.hours[h]!;
  return {
    left: ((g.hx(h) + g.SLOT / 2) / g.W) * 100,
    title: hourRange(h),
    lines: [{ text: youLine(row).replace(`${hourRange(h)} · `, '') }, { text: modelLine(row, base, clockUnit(c)) }, { text: toolLine(row) }],
  };
}

function Ticks({ g, y, h, max }: { g: ReturnType<typeof layout>; y: number; h: number; max: number }) {
  return (
    <>
      <line class="cgrid" x1={g.X0} x2={g.R} y1={y + TITLE} y2={y + TITLE} />
      <text class="clab" x={g.X0 - 8} y={y + TITLE + 4} text-anchor="end">{max}</text>
      <text class="clab" x={g.X0 - 8} y={y + h + 4} text-anchor="end">0</text>
    </>
  );
}

export function HoursDiagram({ clock }: { clock: ClockReport }) {
  const narrow = useNarrow();
  const g = layout(narrow);
  const { W, X0, SLOT, BAR, R, hx } = g;
  const { tip, show, hide } = useTip();
  const active = useSignal<number | null>(null);
  const base = thinkBaseline(clock.hours);
  const set = (h: number | null) => {
    active.value = h;
    if (h === null) hide();
    else show(hourTip(clock, h, base, g));
  };
  const hourAt = (e: PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    return Math.max(0, Math.min(23, Math.floor(((e.clientX - r.left) / r.width * W - X0) / SLOT)));
  };
  const onKey = (e: KeyboardEvent) => {
    const at = active.value ?? -1;
    const next = e.key === 'ArrowRight' ? Math.min(23, at + 1) : e.key === 'ArrowLeft' ? Math.max(0, at < 0 ? 0 : at - 1) : e.key === 'Home' ? 0 : e.key === 'End' ? 23 : null;
    if (next === null) return;
    e.preventDefault();
    set(next);
  };
  const youMax = niceMax(Math.max(0, ...clock.hours.map((h) => (h.prompts < FAINT_BELOW ? 0 : per100(h.corrections, h.prompts) ?? 0))));
  const span = thinkSpan(clock.hours, base);
  const swings = clock.hours.map((h) => thinkPercent(h, base)).flatMap((p) => (p === null ? [] : [p - 100]));
  const cutUp = swings.some((d) => d > span), cutDown = swings.some((d) => d < -span);
  const toolMax = niceMax(Math.max(0, ...clock.hours.map((h) => per100(h.tool_errors, h.tool_calls) ?? 0)));
  const { you, model, tool } = g.strips;
  const yTop = (s: { y: number }) => s.y + TITLE;
  const half = (model.h - TITLE) / 2;
  const modelMid = yTop(model) + half;
  const bands = clock.bad.flatMap(bandSpans);
  const colX = (h: number) => hx(h) + (SLOT - BAR) / 2;
  return (
    <div class="cwrap cfocus" role="slider" tabIndex={0} aria-label="Your hours, one column per hour of the day. Use the arrow keys to read each hour." aria-orientation="horizontal" aria-valuemin={0} aria-valuemax={23} aria-valuenow={active.value ?? 0} aria-valuetext={active.value === null ? undefined : hourRange(active.value)} onFocus={() => { if (active.value === null) set(0); }} onBlur={() => set(null)} onKeyDown={onKey}>
      <svg class="cdiagram" viewBox={`0 0 ${W} ${g.H}`} aria-hidden="true" onPointerMove={(e) => set(hourAt(e))} onPointerLeave={(e) => { if (document.activeElement !== e.currentTarget.parentElement) set(null); }}>
        {bands.map((b, i) => (
          [you, model, tool].map((s) => <rect key={`${i}-${s.y}`} class="cband" x={hx(b.from)} width={(b.to - b.from) * SLOT} y={yTop(s)} height={s.h - TITLE} />)
        ))}
        {active.value !== null && <rect class="cact" x={hx(active.value)} width={SLOT} y={BANDS + TITLE} height={tool.y + tool.h - BANDS - TITLE} />}

        {clock.bad.map((b) => {
          const first = bandSpans(b)[0];
          if (!first) return null;
          // a label sits at its band's start, or ends at the band's end when it would run off the right edge
          const right = hx(first.from) + (narrow ? 120 : 160) > W;
          return <text key={`${b.from_h}-${b.to_h}`} class="cband-label" x={right ? Math.min(hx(first.to), R) : hx(first.from)} y={14} text-anchor={right ? 'end' : 'start'}>{bandLabel(b)}</text>;
        })}

        <text class="cstrip" x={X0} y={you.y + 14}>{narrow ? 'YOU · corrections per 100' : 'YOU · corrections per 100 prompts'}</text>
        <Ticks g={g} y={you.y} h={you.h} max={youMax} />
        {clock.hours.map((h) => {
          const r = per100(h.corrections, h.prompts);
          if (r === null) return null;
          const faint = h.prompts < FAINT_BELOW;
          const top = you.y + you.h - (Math.min(r, youMax) / youMax) * (you.h - TITLE);
          const x = colX(h.h);
          return (
            <g key={h.h}>
              <path class={faint ? 'cbar faint' : 'cbar'} d={colPath(x, BAR, you.y + you.h, top)} />
              {faint && <line class="ccap" x1={x} x2={x + BAR} y1={top} y2={top} />}
            </g>
          );
        })}
        <line class="cbase" x1={X0} x2={R} y1={you.y + you.h} y2={you.y + you.h} />

        <text class="cstrip" x={X0} y={model.y + 14}>{narrow ? 'MODEL · thinking vs usual' : 'THE MODEL · thinking against its usual'}</text>
        <line class="cgrid" x1={X0} x2={R} y1={yTop(model)} y2={yTop(model)} />
        <text class="clab" x={X0 - 8} y={yTop(model) + 4} text-anchor="end">{cutUp ? '≥' : ''}{100 + span}%</text>
        <text class="clab" x={X0 - 8} y={modelMid + 4} text-anchor="end">100%</text>
        <text class="clab" x={X0 - 8} y={model.y + model.h + 4} text-anchor="end">{cutDown ? '≤' : ''}{100 - span}%</text>
        {clock.hours.map((h) => {
          const p = thinkPercent(h, base);
          if (p === null) return null;
          const dev = Math.max(-span, Math.min(span, p - 100));
          const end = modelMid - (dev / span) * half;
          const x = colX(h.h);
          return (
            <g key={h.h}>
              <path class={h.think_n < FAINT_BELOW ? 'cbar faint' : 'cbar'} d={colPath(x, BAR, modelMid, end)} />
              {Math.abs(p - 100) > span && <line class="ccut" x1={x - 2} x2={x + BAR + 2} y1={end} y2={end} />}
              <line class="ctick" x1={x} x2={x + BAR} y1={modelMid} y2={modelMid} />
            </g>
          );
        })}
        <line class="cbase" x1={X0} x2={R} y1={modelMid} y2={modelMid} />

        <text class="cstrip" x={X0} y={tool.y + 14}>{narrow ? 'TOOL ERRORS · per 100 calls' : 'TOOL ERRORS · failed calls per 100'}</text>
        <Ticks g={g} y={tool.y} h={tool.h} max={toolMax} />
        {clock.hours.map((h) => {
          const r = per100(h.tool_errors, h.tool_calls);
          if (r === null) return null;
          const top = tool.y + tool.h - (Math.min(r, toolMax) / toolMax) * (tool.h - TITLE);
          return <path key={h.h} class="cbar" d={colPath(colX(h.h), BAR, tool.y + tool.h, top)} />;
        })}
        <line class="cbase" x1={X0} x2={R} y1={tool.y + tool.h} y2={tool.y + tool.h} />

        {Array.from({ length: 8 }, (_, i) => i * 3).map((h) => (
          <text key={h} class="clab" x={hx(h) + SLOT / 2} y={tool.y + tool.h + 20} text-anchor="middle">{String(h).padStart(2, '0')}</text>
        ))}
        <text class="clab" x={R} y={tool.y + tool.h + 38} text-anchor="end">local time</text>
      </svg>
      <Tip tip={tip} />
    </div>
  );
}

/** Corrections per 100 prompts by time since your last break: four columns, each its own hover and focus target. */
export function BreakChart({ rows }: { rows: ClockBand[] }) {
  const { tip, show, hide } = useTip();
  const w = useNarrow() ? 320 : 420, h = 150, x0 = 40, top = 22, bottom = 118;
  const rates = rows.map((r) => per100(r.corrections, r.prompts));
  const max = niceMax(Math.max(0, ...rates.map((r) => r ?? 0)));
  const slot = (w - x0 - 8) / Math.max(1, rows.length);
  const bw = Math.min(24, slot - 12);
  return (
    <div class="cwrap cbreak">
      <svg viewBox={`0 0 ${w} ${h}`} role="group" aria-label="Corrections per 100 prompts by time since your last break">
        <line class="cgrid" x1={x0} x2={w - 8} y1={top} y2={top} />
        <text class="clab" x={x0 - 8} y={top + 4} text-anchor="end">{max}</text>
        <text class="clab" x={x0 - 8} y={bottom + 4} text-anchor="end">0</text>
        {rows.map((r, i) => {
          const rate = rates[i]!;
          const cx = x0 + slot * i + slot / 2;
          const y = rate === null ? bottom : bottom - (Math.min(rate, max) / max) * (bottom - top);
          const tipOf = () => show({ left: (cx / w) * 100, title: `since your last break: ${r.band}`, lines: [{ text: breakLine(r).replace(`${r.band} since a break · `, '') }] });
          return (
            <g key={r.band}>
              <path class="cbar" d={colPath(cx - bw / 2, bw, bottom, y)} />
              {rate !== null && <text class="cval" x={cx} y={y - 6} text-anchor="middle">{rate1(rate)}</text>}
              <text class="clab" x={cx} y={bottom + 20} text-anchor="middle">{r.band}</text>
              <rect class="chit" x={cx - slot / 2} y={top - 10} width={slot} height={h - top} tabIndex={0} aria-label={breakLine(r)} onPointerMove={tipOf} onPointerLeave={hide} onFocus={tipOf} onBlur={hide} />
            </g>
          );
        })}
        <line class="cbase" x1={x0} x2={w - 8} y1={bottom} y2={bottom} />
      </svg>
      <Tip tip={tip} />
    </div>
  );
}

/** The numbers behind the diagram: one row per hour, then one per break band. */
export function HoursTable({ clock }: { clock: ClockReport }) {
  const base = thinkBaseline(clock.hours);
  const fmt = (n: number | null) => (n === null ? '–' : rate1(n));
  return (
    <>
      <table class="ctable">
        <caption>Hours of the day, local time</caption>
        <thead><tr><th>Hour</th><th>Prompts</th><th>Corrections</th><th>Per 100</th><th>Thinking vs usual</th><th>Tool errors per 100</th></tr></thead>
        <tbody>
          {clock.hours.map((h) => {
            const p = thinkPercent(h, base);
            return (
              <tr key={h.h} class={h.prompts < FAINT_BELOW ? 'faint' : ''}>
                <th scope="row">{hourRange(h.h)}</th><td>{h.prompts}</td><td>{h.corrections}</td><td>{fmt(per100(h.corrections, h.prompts))}</td>
                <td>{p === null ? '–' : `${Math.round(p)}%`}</td><td>{fmt(per100(h.tool_errors, h.tool_calls))}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <table class="ctable">
        <caption>Since your last break</caption>
        <thead><tr><th>Band</th><th>Prompts</th><th>Corrections</th><th>Per 100</th></tr></thead>
        <tbody>
          {clock.since_break.map((b) => <tr key={b.band}><th scope="row">{b.band}</th><td>{b.prompts}</td><td>{b.corrections}</td><td>{fmt(per100(b.corrections, b.prompts))}</td></tr>)}
        </tbody>
      </table>
    </>
  );
}
