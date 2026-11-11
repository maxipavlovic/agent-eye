import type { Worked } from '../../api/types';

const DAY = 86_400_000;

/** A `YYYY-MM-DD` day as a number of days since 1970-01-01, the same for every time zone. */
export function dayNumber(day: string): number {
  const [y, m, d] = day.split('-').map(Number);
  return Math.round(Date.UTC(y!, m! - 1, d!) / DAY);
}

/** The inverse of `dayNumber`. */
export function dayName(n: number): string {
  return new Date(n * DAY).toISOString().slice(0, 10);
}

/** The local calendar day of a time, as a day number. */
function localDay(ms: number): number {
  const d = new Date(ms);
  return Math.round(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / DAY);
}

/** "7 h 40 m", "25 m", "under 1 m". */
export function activeText(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60_000);
  if (minutes === 0) return 'under 1 m';
  const h = Math.floor(minutes / 60);
  return h === 0 ? `${minutes} m` : `${h} h ${String(minutes % 60).padStart(2, '0')} m`;
}

const shortDate = (n: number) => new Date(n * DAY).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', timeZone: 'UTC' });

/** "today", "yesterday" or "3 Nov": the local day of a time. */
function dayWord(ms: number, now: number): string {
  const ago = localDay(now) - localDay(ms);
  return ago === 0 ? 'today' : ago === 1 ? 'yesterday' : shortDate(localDay(ms));
}

/** "today 14:20", "yesterday 09:05", "3 Nov 09:05". */
function when(ms: number, now: number): string {
  const clock = new Date(ms).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  return `${dayWord(ms, now)} ${clock}`;
}

/** The line next to WORKED ON. A session worked on one day says just that day and the active time. */
export function workedText(w: Worked, now = Date.now()): string {
  if (w.span_days === 1) return `${dayWord(w.last_at, now)} · ${activeText(w.active_ms)} active`;
  const days = `${w.days_count} ${w.days_count === 1 ? 'day' : 'days'} over ${w.span_days}`;
  return `${days} · ${activeText(w.active_ms)} active · last ${when(w.last_at, now)}`;
}

/** One cell of the strip: a calendar day, filled 0 (no work) to 4 by how much was done. */
export interface Cell {
  day: string;
  level: 0 | 1 | 2 | 3 | 4;
  title: string;
}

/** The cells from the first to the last day of the strip, each day with its date, prompts and time. */
export function stripCells(w: Worked): Cell[] {
  const by = new Map(w.days.map((d) => [d.day, d]));
  const most = Math.max(1, ...w.days.map((d) => d.active_ms));
  const cells: Cell[] = [];
  for (let n = dayNumber(w.first_day); n <= dayNumber(w.last_day); n++) {
    const day = dayName(n);
    const d = by.get(day);
    const date = new Date(n * DAY).toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short', timeZone: 'UTC' });
    cells.push({
      day,
      level: d ? (Math.max(1, Math.ceil((4 * d.active_ms) / most)) as 1 | 2 | 3 | 4) : 0,
      title: d ? `${date} · ${d.prompts} ${d.prompts === 1 ? 'prompt' : 'prompts'} · ${activeText(d.active_ms)} active` : `${date} · no work`,
    });
  }
  return cells;
}
