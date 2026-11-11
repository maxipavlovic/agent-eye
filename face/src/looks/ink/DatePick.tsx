import { useEffect, useRef, useState } from 'preact/hooks';
import { Pix } from '../../pixel/Pix';
import { INK_CC } from './art';

const pad = (n: number) => String(n).padStart(2, '0');

/** `2026-11-20`: the form's date, and what the engine reads. */
export function dateValue(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

const parse = (iso: string) => {
  const [y, m, d] = iso.split('-').map(Number);
  return new Date(y!, m! - 1, d!);
};

export function addDays(iso: string, n: number): string {
  const d = parse(iso);
  d.setDate(d.getDate() + n);
  return dateValue(d);
}

/** The same day `n` months on, or the month's last day when it is shorter (31 Jan + 1 = 28 Feb). */
export function addMonths(iso: string, n: number): string {
  const d = parse(iso);
  const day = d.getDate();
  d.setDate(1);
  d.setMonth(d.getMonth() + n);
  d.setDate(Math.min(day, new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate()));
  return dateValue(d);
}

/** The weeks of the month of `iso`, Monday first; the cells before the 1st and after the last day are null. */
export function monthWeeks(iso: string): Array<Array<string | null>> {
  const d = parse(iso);
  const y = d.getFullYear();
  const m = d.getMonth();
  const days = new Date(y, m + 1, 0).getDate();
  const cells: Array<string | null> = Array((new Date(y, m, 1).getDay() + 6) % 7).fill(null);
  for (let i = 1; i <= days; i++) cells.push(dateValue(new Date(y, m, i)));
  while (cells.length % 7) cells.push(null);
  const weeks = [];
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
  return weeks;
}

/** A typed time as `HH:MM` (24 h): `9`, `930`, `9:30` and `09:30` all read; null when it is not a time. */
export function parseTime(s: string): string | null {
  const m = /^(\d{1,2})(?::?(\d{2}))?$/.exec(s.trim());
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2] ?? 0);
  return h > 23 || min > 59 ? null : `${pad(h)}:${pad(min)}`;
}

/** The next quarter hour up or down from `t`, stopping at 00:00 and 23:45 so the date never moves; `t` counts as 09:00 when it is not a time. */
export function stepTime(t: string, dir: 1 | -1): string {
  const [h, m] = (parseTime(t) ?? '09:00').split(':').map(Number);
  const at = h! * 60 + m!;
  const next = dir > 0 ? Math.floor(at / 15) * 15 + 15 : Math.ceil(at / 15) * 15 - 15;
  const w = Math.min(Math.max(next, 0), 23 * 60 + 45);
  return `${pad(Math.floor(w / 60))}:${pad(w % 60)}`;
}

const WEEKDAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
const UP = ['..#..', '.###.', '#####'];
const DOWN = ['#####', '.###.', '..#..'];
/** The quick times next to the field. */
export const TIME_CHIPS = ['09:00', '12:00', '15:00', '18:00'];

const KEY_STEP: Record<string, (iso: string) => string> = {
  ArrowLeft: (d) => addDays(d, -1),
  ArrowRight: (d) => addDays(d, 1),
  ArrowUp: (d) => addDays(d, -7),
  ArrowDown: (d) => addDays(d, 7),
  PageUp: (d) => addMonths(d, -1),
  PageDown: (d) => addMonths(d, 1),
  Home: (d) => addDays(d, -((parse(d).getDay() + 6) % 7)),
  End: (d) => addDays(d, 6 - ((parse(d).getDay() + 6) % 7)),
};

/**
 * A month of days to pick one from, Monday first. Days before `min` are dimmed and cannot be picked.
 * Arrows move a day or a week, PageUp and PageDown a month, Home and End to the week's ends; Enter picks.
 */
export function DatePicker({ value, min, onChange, labelId }: { value: string; min: string; onChange: (iso: string) => void; labelId: string }) {
  const [cursor, setCursor] = useState(value < min ? min : value);
  const grid = useRef<HTMLTableElement>(null);
  // the day the keys moved to: it takes the focus once it is drawn
  const moved = useRef<string | null>(null);
  useEffect(() => {
    if (moved.current !== cursor) return;
    moved.current = null;
    grid.current?.querySelector<HTMLButtonElement>('button[tabindex="0"]')?.focus();
  }, [cursor]);
  const go = (iso: string, focus: boolean) => {
    const to = iso < min ? min : iso;
    moved.current = focus ? to : null;
    setCursor(to);
  };
  const onKey = (e: KeyboardEvent) => {
    const step = KEY_STEP[e.key];
    if (!step || e.altKey || e.metaKey || e.ctrlKey) return;
    // the board's own arrow keys walk the cards; here they move the day
    e.preventDefault();
    e.stopPropagation();
    go(step(cursor), true);
  };
  const month = parse(cursor).toLocaleDateString('en-GB', { month: 'long', year: 'numeric' }).toUpperCase();
  const today = dateValue(new Date());
  const monthId = `${labelId}-month`;
  return (
    <div class="dpick">
      <div class="dphead">
        <button type="button" aria-label="Previous month" title="Previous month (PageUp)" disabled={addMonths(cursor, -1).slice(0, 7) < min.slice(0, 7)} onClick={() => go(addMonths(cursor, -1), false)}>‹</button>
        <span id={monthId} aria-live="polite">{month}</span>
        <button type="button" aria-label="Next month" title="Next month (PageDown)" onClick={() => go(addMonths(cursor, 1), false)}>›</button>
      </div>
      <table ref={grid} class="dpgrid" role="grid" aria-labelledby={`${labelId} ${monthId}`} onKeyDown={onKey}>
        <thead>
          <tr>{WEEKDAYS.map((w) => <th key={w} scope="col" abbr={w}>{w.slice(0, 2)}</th>)}</tr>
        </thead>
        <tbody>
          {monthWeeks(cursor).map((week, i) => (
            <tr key={i}>
              {week.map((d, j) => (
                <td key={j} role="gridcell" aria-selected={d ? d === value : undefined}>
                  {d && (
                    <button
                      type="button"
                      class={`${d === value ? 'on' : ''}${d === today ? ' today' : ''}`}
                      tabIndex={d === cursor ? 0 : -1}
                      disabled={d < min}
                      aria-current={d === today ? 'date' : undefined}
                      aria-label={parse(d).toLocaleDateString('en-GB', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })}
                      onClick={() => { go(d, false); onChange(d); }}
                    >{Number(d.slice(8))}</button>
                  )}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** A 24 h HH:MM field: ▲ ▼ or the arrow keys step a quarter hour, and a few quick times below it. */
export function TimeField({ value, onChange, invalid, id }: { value: string; onChange: (t: string) => void; invalid: boolean; id: string }) {
  const step = (dir: 1 | -1) => onChange(stepTime(value, dir));
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
    e.preventDefault();
    e.stopPropagation();
    step(e.key === 'ArrowUp' ? 1 : -1);
  };
  return (
    <>
      <div class="tfield">
        <input
          id={id}
          name="time"
          inputMode="numeric"
          autocomplete="off"
          maxLength={5}
          placeholder="09:00"
          aria-invalid={invalid || undefined}
          aria-describedby={invalid ? `${id}-err ${id}-hint` : `${id}-hint`}
          value={value}
          onInput={(e) => onChange(e.currentTarget.value)}
          onBlur={() => { const t = parseTime(value); if (t) onChange(t); }}
          onKeyDown={onKey}
        />
        <span class="tstep">
          <button type="button" tabIndex={-1} aria-label="15 minutes later" onClick={() => step(1)}><Pix rows={UP} pal={INK_CC} scale={2} /></button>
          <button type="button" tabIndex={-1} aria-label="15 minutes earlier" onClick={() => step(-1)}><Pix rows={DOWN} pal={INK_CC} scale={2} /></button>
        </span>
      </div>
      <div class="fchips tchips" role="group" aria-label="Quick times">
        {TIME_CHIPS.map((t) => <button key={t} type="button" aria-pressed={parseTime(value) === t} onClick={() => onChange(t)}>{t}</button>)}
      </div>
      <p class="thint" id={`${id}-hint`}>24 h. ▲ ▼ or the arrow keys step 15 minutes.</p>
    </>
  );
}
