import { useSignal } from '@preact/signals';
import type { ClockReport, ClockBand } from '../api/types';
import { colPath, Tip, useTip } from './chartParts';
import {
  bandLabel, bandSpans, breakLine, FAINT_BELOW, hourRange, modelLine, niceMax, per100, rate1, thinkBaseline, thinkPercent, thinkSpan, toolLine, youLine,
} from './clockModel';

// The HOURS diagram: one 24-hour axis in local time, shared by three strips (you, the model, tool errors), each with
// its own unit and no second scale. All three are read by the same hover or focus: one hour, one tooltip.

const W = 720;
const X0 = 48;
const SLOT = (W - X0 - 8) / 24;
const BAR = Math.min(20, SLOT - 6);
const TITLE = 22;
const STRIPS = { you: { y: 0, h: 120 }, model: { y: 160, h: 96 }, tool: { y: 296, h: 44 } };
const H = 372;
const hx = (h: number) => X0 + h * SLOT;

/** The tooltip of an hour: what you did, what the model did, what the tools did. */
function hourTip(c: ClockReport, h: number, base: number | null) {
  const row = c.hours[h]!;
  const unit = c.agents[0]?.think ?? null;
  return {
    left: ((hx(h) + SLOT / 2) / W) * 100,
    title: hourRange(h),
    lines: [{ key: 'var(--c1)', text: youLine(row).replace(`${hourRange(h)} · `, '') }, { key: 'var(--c1)', text: modelLine(row, base, unit) }, { key: 'var(--c1)', text: toolLine(row) }],
  };
}

function Ticks({ y, h, max, fmt }: { y: number; h: number; max: number; fmt: (n: number) => string }) {
  return (
    <>
      <line class="cgrid" x1={X0} x2={W - 8} y1={y + TITLE} y2={y + TITLE} />
      <text class="clab" x={X0 - 8} y={y + TITLE + 4} text-anchor="end">{fmt(max)}</text>
      <text class="clab" x={X0 - 8} y={y + h + 4} text-anchor="end">{fmt(0)}</text>
    </>
  );
}

export function HoursDiagram({ clock }: { clock: ClockReport }) {
  const { tip, show, hide } = useTip();
  const active = useSignal<number | null>(null);
  const base = thinkBaseline(clock.hours);
  const set = (h: number | null) => {
    active.value = h;
    if (h === null) hide();
    else show(hourTip(clock, h, base));
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
  const toolMax = niceMax(Math.max(0, ...clock.hours.map((h) => per100(h.tool_errors, h.tool_calls) ?? 0)));
  const you = STRIPS.you, model = STRIPS.model, tool = STRIPS.tool;
  const yTop = (s: { y: number }) => s.y + TITLE;
  const modelMid = yTop(model) + (model.h - TITLE) / 2;
  const bands = clock.bad.flatMap(bandSpans);
  const first = clock.bad[0] ? bandLabel(clock.bad[0]) : '';
  const band0 = clock.bad[0] ? bandSpans(clock.bad[0])[0] : undefined;
  return (
    <div class="cwrap cfocus" role="group" tabIndex={0} aria-label="Your hours, one column per hour of the day. Use the arrow keys to read each hour." onFocus={() => { if (active.value === null) set(0); }} onBlur={() => set(null)} onKeyDown={onKey}>
      <svg class="cdiagram" viewBox={`0 0 ${W} ${H}`} aria-hidden="true" onPointerMove={(e) => set(hourAt(e))} onPointerLeave={(e) => { if (document.activeElement !== e.currentTarget.parentElement) set(null); }}>
        {bands.map((b, i) => (
          [you, model, tool].map((s) => <rect key={`${i}-${s.y}`} class="cband" x={hx(b.from)} width={(b.to - b.from) * SLOT} y={yTop(s)} height={s.h - TITLE} />)
        ))}
        {active.value !== null && <rect class="cact" x={hx(active.value)} width={SLOT} y={TITLE} height={tool.y + tool.h - TITLE} />}

        <text class="cstrip" x={X0} y={14}>YOU · corrections per 100 prompts</text>
        {band0 && <text class="cband-label" x={Math.min(Math.max(hx(band0.from), 330), W - 120)} y={14}>{first}</text>}
        <Ticks y={you.y} h={you.h} max={youMax} fmt={(n) => String(n)} />
        {clock.hours.map((h) => {
          const r = per100(h.corrections, h.prompts);
          if (r === null) return null;
          const faint = h.prompts < FAINT_BELOW;
          const top = you.y + you.h - (Math.min(r, youMax) / youMax) * (you.h - TITLE);
          const x = hx(h.h) + (SLOT - BAR) / 2;
          return (
            <g key={h.h}>
              <path class={faint ? 'cbar faint' : 'cbar'} d={colPath(x, BAR, you.y + you.h, top)} />
              {faint && <line class="ccap" x1={x} x2={x + BAR} y1={top} y2={top} />}
            </g>
          );
        })}
        <line class="cbase" x1={X0} x2={W - 8} y1={you.y + you.h} y2={you.y + you.h} />

        <text class="cstrip" x={X0} y={model.y + 14}>THE MODEL · thinking against its usual</text>
        <line class="cgrid" x1={X0} x2={W - 8} y1={yTop(model)} y2={yTop(model)} />
        <text class="clab" x={X0 - 8} y={yTop(model) + 4} text-anchor="end">{100 + span}%</text>
        <text class="clab" x={X0 - 8} y={modelMid + 4} text-anchor="end">100%</text>
        <text class="clab" x={X0 - 8} y={model.y + model.h + 4} text-anchor="end">{100 - span}%</text>
        {clock.hours.map((h) => {
          const p = thinkPercent(h, base);
          if (p === null) return null;
          const half = (model.h - TITLE) / 2;
          const end = modelMid - ((p - 100) / span) * half;
          return <path key={h.h} class="cbar" d={colPath(hx(h.h) + (SLOT - BAR) / 2, BAR, modelMid, end)} />;
        })}
        <line class="cbase" x1={X0} x2={W - 8} y1={modelMid} y2={modelMid} />

        <text class="cstrip" x={X0} y={tool.y + 14}>TOOL ERRORS · failed calls per 100</text>
        <Ticks y={tool.y} h={tool.h} max={toolMax} fmt={(n) => String(n)} />
        {clock.hours.map((h) => {
          const r = per100(h.tool_errors, h.tool_calls);
          if (r === null) return null;
          const top = tool.y + tool.h - (Math.min(r, toolMax) / toolMax) * (tool.h - TITLE);
          return <path key={h.h} class="cbar" d={colPath(hx(h.h) + (SLOT - BAR) / 2, BAR, tool.y + tool.h, top)} />;
        })}
        <line class="cbase" x1={X0} x2={W - 8} y1={tool.y + tool.h} y2={tool.y + tool.h} />

        {Array.from({ length: 8 }, (_, i) => i * 3).map((h) => (
          <text key={h} class="clab" x={hx(h) + SLOT / 2} y={tool.y + tool.h + 20} text-anchor="middle">{String(h).padStart(2, '0')}</text>
        ))}
        <text class="clab" x={W - 8} y={tool.y + tool.h + 38} text-anchor="end">local time</text>
      </svg>
      <Tip tip={tip} />
    </div>
  );
}

/** Corrections per 100 prompts by time since your last break: four columns, each its own hover and focus target. */
export function BreakChart({ rows }: { rows: ClockBand[] }) {
  const { tip, show, hide } = useTip();
  const w = 420, h = 150, x0 = 40, top = 22, bottom = 118;
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
          const tipOf = () => show({ left: (cx / w) * 100, title: `since your last break: ${r.band}`, lines: [{ key: 'var(--c1)', text: breakLine(r).replace(`${r.band} since a break · `, '') }] });
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
