import type { ClockAgent, ClockBad, ClockRange, ClockReport, ClockBand, ClockHour } from '../api/types';

/** An hour with fewer prompts than this says too little about the rate: it is drawn faint and the tooltip says so. */
export const FAINT_BELOW = 20;

const pad = (h: number) => String(h).padStart(2, '0');

/** `21:00–22:00`; the last hour of the day ends at 24:00. */
export function hourRange(h: number): string {
  return `${pad(h)}:00–${pad(h + 1)}:00`;
}

/** `13.0`: a rate per 100, one decimal. */
export const rate1 = (x: number) => (Math.round(x * 10) / 10).toFixed(1);

/** Events per 100 of something; null when there is nothing to divide by. */
export function per100(n: number, of: number): number | null {
  return of > 0 ? (n / of) * 100 : null;
}

/** The pieces of a band as drawn on a 0-24 axis: a band that crosses midnight is two (the evening and the small hours). */
export function bandSpans(b: ClockRange): Array<{ from: number; to: number }> {
  if (b.from_h === b.to_h) return [];
  if (b.from_h < b.to_h) return [{ from: b.from_h, to: b.to_h }];
  const spans = [{ from: b.from_h, to: 24 }];
  if (b.to_h > 0) spans.push({ from: 0, to: b.to_h });
  return spans;
}

/** The band's name as the engine gives it, so the sentence, the hint and the chart say the same ("your evening"). */
export const bandLabel = (b: ClockBad) => `your ${b.label}`;

/** The thinking measure the model strip is drawn against: the median of the hours that have one. */
export function thinkBaseline(hours: ClockHour[]): number | null {
  const v = hours.flatMap((h) => (h.think === null ? [] : [h.think])).sort((a, b) => a - b);
  if (v.length === 0) return null;
  const m = v.length >> 1;
  return v.length % 2 ? v[m]! : (v[m - 1]! + v[m]!) / 2;
}

/** Thinking in an hour as a share of the usual (100 = usual), or null without data. */
export function thinkPercent(h: ClockHour, base: number | null): number | null {
  return h.think === null || base === null || base <= 0 ? null : (h.think / base) * 100;
}

/** The most the model strip shows either side of 100; a bigger swing is cut at the edge and marked. */
export const THINK_CAP = 50;

/** The half-height of the model strip: at least 25 points around 100, then the biggest swing rounded up to 25, never past the cap. */
export function thinkSpan(hours: ClockHour[], base: number | null): number {
  const swings = hours.map((h) => thinkPercent(h, base)).flatMap((p) => (p === null ? [] : [Math.abs(p - 100)]));
  return Math.min(THINK_CAP, Math.max(25, Math.ceil(Math.max(0, ...swings) / 25) * 25));
}

/** The scale top of a rate strip: the biggest rate rounded up to a clean number (5, 10, 20, 25, 50, 100). */
export function niceMax(max: number): number {
  for (const t of [5, 10, 20, 25, 50, 100]) if (max <= t) return t;
  return Math.ceil(max / 100) * 100;
}

/** The unit the thinking measure is in, in words for a tooltip. */
const THINK_UNIT: Record<string, string> = { signature: 'thinking signature length', reasoning_tokens: 'reasoning tokens', thinking_chars: 'thinking characters' };
export const thinkUnit = (u: string | null | undefined) => (u ? THINK_UNIT[u] ?? u : 'thinking depth');

const count = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The tooltip line about the user: "21:00–22:00 · 14 corrections in 108 prompts (13.0 per 100)". */
export function youLine(h: ClockHour): string {
  const base = `${hourRange(h.h)} · `;
  if (h.prompts === 0) return `${base}no prompts`;
  const r = per100(h.corrections, h.prompts)!;
  const line = `${base}${count(h.corrections, 'correction')} in ${count(h.prompts, 'prompt')} (${rate1(r)} per 100)`;
  return h.prompts < FAINT_BELOW ? `${line}, too few prompts to read much into (under ${FAINT_BELOW})` : line;
}

/** The unit of the agent whose thinking the hours carry (`think_agent`), or null. */
export function clockUnit(c: ClockReport): string | null {
  return c.agents.find((a) => a.agent === c.think_agent)?.think_unit ?? null;
}

/** The tooltip line about the model: "thinking 92% of usual (thinking signature length, median of 800)". */
export function modelLine(h: ClockHour, base: number | null, unit: string | null): string {
  const p = thinkPercent(h, base);
  if (p === null) return 'thinking: no data this hour';
  const line = `thinking ${Math.round(p)}% of usual (${thinkUnit(unit)}, median of ${h.think_n})`;
  return h.think_n < FAINT_BELOW ? `${line}, too few to read much into (under ${FAINT_BELOW})` : line;
}

/** The tooltip line about failed tool calls: "20 failed in 900 calls (2.2 per 100)". */
export function toolLine(h: ClockHour): string {
  const r = per100(h.tool_errors, h.tool_calls);
  return r === null ? 'tool calls: none this hour' : `${h.tool_errors} failed in ${count(h.tool_calls, 'call')} (${rate1(r)} per 100)`;
}

/** The tooltip line of a "since your last break" bar. */
export function breakLine(b: ClockBand): string {
  const r = per100(b.corrections, b.prompts);
  if (r === null) return `${b.band} since a break · no prompts`;
  return `${b.band} since a break · ${count(b.corrections, 'correction')} in ${count(b.prompts, 'prompt')} (${rate1(r)} per 100)`;
}

/** What the panel says instead of the diagram, or null when there is a diagram to draw. */
export function clockNote(c: ClockReport): string | null {
  if (!c.enabled) return 'The steering clock is off. Switch it on in Settings, under HOURS.';
  if (c.prompts === 0) return 'Nothing recorded yet. eye notes when you prompt and when the agent finishes, as your sessions run.';
  if (c.verdict.kind === 'too_few') return c.judging === 'on' ? `Not enough data yet: ${count(c.prompts, 'prompt')}, ${count(c.corrections, 'correction')}.` : `${judgingNote(c)} ${c.verdict.text}`;
  return null;
}

/** Why corrections stay at zero when they are not judged (the engine's reason), or null when they are. */
export function judgingNote(c: ClockReport): string | null {
  if (c.judging === 'on') return null;
  return `Corrections are not being judged: ${c.judging_reason ?? 'no agent can judge them'}.`;
}

/** The sentence for prompts the nightly pass has not judged yet, or null (also when nothing is judged at all). */
export function pendingNote(c: ClockReport): string | null {
  return c.judging === 'on' && c.pending > 0 ? `${count(c.pending, 'prompt')} not judged yet: the nightly pass counts them.` : null;
}

/** A ratio against the rest as a change ("+12%", "−7%", "flat"), or "—" when the agent has no such signal. */
export function ratioText(r: number | null): string {
  if (r === null) return '—';
  const pct = Math.round((r - 1) * 100);
  return Math.abs(pct) < 5 ? 'flat' : `${pct > 0 ? '+' : '−'}${Math.abs(pct)}%`;
}

/** One agent's row of the by-agent table; a signal the agent cannot report reads "—", never zero. */
export function agentRow(a: ClockAgent): { prompts: string; corrections: string; thinking: string; toolErrors: string } {
  const has = (s: ClockAgent['signals'][number]) => a.signals.includes(s);
  return {
    prompts: has('prompt') ? String(a.prompts) : '—',
    corrections: has('prompt') ? String(a.corrections) : '—',
    thinking: has('think') ? ratioText(a.think_ratio) : '—',
    toolErrors: has('tool_error') ? ratioText(a.tool_error_ratio) : '—',
  };
}
