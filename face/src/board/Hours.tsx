import { useEffect, useLayoutEffect } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import type { ClockReport } from '../api/types';
import type { Store } from '../store';
import { BreakChart, HoursDiagram, HoursTable } from './ClockChart';
import { ChartFrame } from './chartParts';
import { clockNote, pendingNote } from './clockModel';
import { Sec } from './Sections';

/** Is the HOURS panel open? The header button sets it (and the hint beside it). */
export const hoursPanel = signal(false);

const WEEKS = [{ weeks: 2, label: '2 WEEKS' }, { weeks: 4, label: '4 WEEKS' }, { weeks: 8, label: '8 WEEKS' }];

/** The panel body for a clock that has something to say: the verdict, the diagram, the break chart. */
function Body({ c }: { c: ClockReport }) {
  const table = useSignal(false);
  const pending = pendingNote(c);
  return (
    <>
      <p class="verdict">{c.verdict.text}</p>
      {pending && <p class="umeta">{pending}</p>}
      <Sec t="BY HOUR OF THE DAY">
        <ChartFrame title={`${c.from} to ${c.to}`} table={table.value} onTable={(t) => { table.value = t; }}>
          {table.value ? <HoursTable clock={c} /> : <HoursDiagram clock={c} />}
        </ChartFrame>
      </Sec>
      {!table.value && c.since_break.length > 0 && (
        <Sec t="SINCE YOUR LAST BREAK">
          <ChartFrame title="corrections per 100 prompts" table={false} onTable={() => { table.value = true; }}>
            <BreakChart rows={c.since_break} />
          </ChartFrame>
        </Sec>
      )}
    </>
  );
}

/** The HOURS panel: when you steer well or badly, and whether it is you or the model. */
export function HoursHost({ store }: { store: Store }) {
  const open = hoursPanel.value;
  const weeks = useSignal(4);
  const data = useSignal<ClockReport | null>(null);
  const failed = useSignal(false);
  useEffect(() => {
    if (!open) return;
    let live = true;
    data.value = null;
    failed.value = false;
    void store.clock(weeks.value).then((r) => {
      if (!live) return;
      data.value = r;
      failed.value = r === null;
    });
    return () => { live = false; };
  }, [open, weeks.value]);
  // Esc listens from before the paint, so the first one closes this and not the panel under it
  useLayoutEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopImmediatePropagation();
      hoursPanel.value = false;
    };
    addEventListener('keydown', onKey, true);
    return () => removeEventListener('keydown', onKey, true);
  }, [open]);
  if (!open) return null;
  const close = () => { hoursPanel.value = false; };
  const c = data.value;
  const note = c ? clockNote(c) : null;
  return (
    <>
      <div class="scrim" role="presentation" onClick={close} />
      <aside class="pagewrap" aria-label="Hours">
        <div class="page usage hours">
          <button class="x" onClick={close} aria-label="Close (Esc)">ESC ✕</button>
          <header class="dh"><div><h2>Hours</h2><p class="umeta">When you steer well and when you do not, from timing eye already sees. Never what you type.</p></div></header>
          <div class="acts">
            {WEEKS.map((w) => (
              <button key={w.weeks} class={`btn sm${weeks.value === w.weeks ? '' : ' ghost'}`} aria-pressed={weeks.value === w.weeks} onClick={() => { weeks.value = w.weeks; }}>{w.label}</button>
            ))}
          </div>
          {!c && !failed.value && <p class="state">Loading…<i class="dots" /></p>}
          {failed.value && <p class="state">Not available in this version. Update eye, then reload.</p>}
          {c && note && <p class="state">{note}</p>}
          {c && !note && <Body c={c} />}
        </div>
        <div class="dis" />
      </aside>
    </>
  );
}
