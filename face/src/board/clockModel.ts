import type { ClockReport, ClockRange, ClockBand, ClockHour } from '../api/types';

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

/** The part of the day a stretch of hours starts in, as the label of a bad band ("your evening"). */
export function partOfDay(h: number): 'morning' | 'afternoon' | 'evening' | 'night' {
  if (h >= 5 && h < 12) return 'morning';
  if (h >= 12 && h < 17) return 'afternoon';
  if (h >= 17) return 'evening';
  return 'night';
}

/** Is hour `h` inside the band? A band that ends before it starts crosses midnight; `to_h` is exclusive. */
export function inBand(h: number, b: ClockRange): boolean {
  if (b.from_h === b.to_h) return false;
  return b.from_h < b.to_h ? h >= b.from_h && h < b.to_h : h >= b.from_h || h < b.to_h;
}

/** The pieces of a band as drawn on a 0-24 axis: a band that crosses midnight is two (the evening and the small hours). */
export function bandSpans(b: ClockRange): Array<{ from: number; to: number; label: boolean }> {
  if (b.from_h === b.to_h) return [];
  if (b.from_h < b.to_h) return [{ from: b.from_h, to: b.to_h, label: true }];
  const spans = [{ from: b.from_h, to: 24, label: true }];
  if (b.to_h > 0) spans.push({ from: 0, to: b.to_h, label: false });
  return spans;
}

export const bandLabel = (b: ClockRange) => `your ${partOfDay(b.from_h)}`;

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

/** The half-height of the model strip: at least 25 points around 100, then the biggest swing rounded up to 25. */
export function thinkSpan(hours: ClockHour[], base: number | null): number {
  const swings = hours.map((h) => thinkPercent(h, base)).flatMap((p) => (p === null ? [] : [Math.abs(p - 100)]));
  return Math.max(25, Math.ceil(Math.max(0, ...swings) / 25) * 25);
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

/** The tooltip line about the model: "thinking 92% of usual (thinking signature length, median of 800)". */
export function modelLine(h: ClockHour, base: number | null, unit: string | null): string {
  const p = thinkPercent(h, base);
  if (p === null) return 'thinking: no data this hour';
  return `thinking ${Math.round(p)}% of usual (${thinkUnit(unit)}, median of ${h.think_n})`;
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
  if (c.verdict.kind === 'too_few') return `Not enough data yet: ${count(c.prompts, 'prompt')}, ${count(c.corrections, 'correction')}.`;
  return null;
}

/** The sentence for prompts the nightly pass has not judged yet, or null. */
export function pendingNote(c: ClockReport): string | null {
  return c.pending > 0 ? `${count(c.pending, 'prompt')} not judged yet: the nightly pass counts them.` : null;
}
