import { useEffect } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import type { ClockReport } from '../api/types';
import type { Store } from '../store';
import { BreakChart, HoursDiagram, HoursTable } from './ClockChart';
import { ChartFrame } from './chartParts';
import { agentRow, clockNote, judgingNote, pendingNote } from './clockModel';
import { Sec } from './Sections';
import { usePanelEsc } from './panelEsc';

/** Is the HOURS panel open? The header button sets it (and the hint beside it). */
export const hoursPanel = signal(false);

const WEEKS = [{ weeks: 2, label: '2 WEEKS' }, { weeks: 4, label: '4 WEEKS' }, { weeks: 8, label: '8 WEEKS' }];

/** What each agent says on its own: its prompts and corrections, and its model's thinking and tool errors in the bad hours; "—" where it has no such signal. */
function AgentTable({ c, store }: { c: ClockReport; store: Store }) {
  const name = (id: string) => store.agents.value.find((a) => a.id === id)?.name ?? id;
  return (
    <table class="ctable">
      <caption>Thinking and tool errors are against the same agent's other hours; "—" means it does not report that.</caption>
      <thead><tr><th>Agent</th><th>Prompts</th><th>Corrections</th><th>Thinking</th><th>Tool errors</th></tr></thead>
      <tbody>
        {c.agents.map((a) => {
          const r = agentRow(a);
          return <tr key={a.agent}><th scope="row">{name(a.agent)}</th><td>{r.prompts}</td><td>{r.corrections}</td><td>{r.thinking}</td><td>{r.toolErrors}</td></tr>;
        })}
      </tbody>
    </table>
  );
}

/** The panel body for a clock that has something to say: the verdict, the diagram, the break chart, each agent. */
function Body({ c, store }: { c: ClockReport; store: Store }) {
  const table = useSignal(false);
  const pending = judgingNote(c) ?? pendingNote(c);
  return (
    <>
      <p class="verdict">{c.verdict.text}</p>
      {pending && <p class="umeta">{pending}</p>}
      <Sec t="BY HOUR OF THE DAY">
        <ChartFrame title={`${c.from} to ${c.to}`} table={table.value} onTable={(t) => { table.value = t; }}>
          {table.value ? <HoursTable clock={c} /> : <HoursDiagram clock={c} toolAgent={c.agents.length > 1 && c.tool_agent ? store.agents.value.find((a) => a.id === c.tool_agent)?.short ?? c.tool_agent : undefined} />}
        </ChartFrame>
      </Sec>
      {!table.value && c.since_break.length > 0 && (
        <Sec t="SINCE YOUR LAST BREAK">
          <ChartFrame title="corrections per 100 prompts">
            <BreakChart rows={c.since_break} />
          </ChartFrame>
        </Sec>
      )}
      {c.agents.length > 0 && <Sec t="BY AGENT"><AgentTable c={c} store={store} /></Sec>}
    </>
  );
}

/** The HOURS panel: when you steer well or badly, and whether it is you or the model. */
export function HoursHost({ store }: { store: Store }) {
  const open = hoursPanel.value;
  const weeks = useSignal(4);
  const data = useSignal<ClockReport | null>(null);
  const missing = useSignal(false);
  const broken = useSignal('');
  useEffect(() => {
    if (!open) return;
    let live = true;
    missing.value = false;
    broken.value = '';
    // a new range keeps the chart on screen until the answer comes; a new opening starts clean
    store.clock(weeks.value).then((r) => {
      if (!live) return;
      data.value = r;
      missing.value = r === null;
    }, (e: unknown) => {
      if (live) broken.value = e instanceof Error ? e.message : 'unknown error';
    });
    return () => { live = false; };
  }, [open, weeks.value]);
  useEffect(() => { data.value = null; }, [open]);
  usePanelEsc(hoursPanel);
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
          {!c && !missing.value && !broken.value && <p class="state">Loading…<i class="dots" /></p>}
          {missing.value && <p class="state">Not available in this version. Update eye, then reload.</p>}
          {broken.value && <p class="state">Could not load the hours: {broken.value}</p>}
          {c && note && <p class="state">{note}</p>}
          {c && !note && <Body c={c} store={store} />}
        </div>
        <div class="dis" />
      </aside>
    </>
  );
}
