import { useEffect } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import type { Store } from '../../store';
import type { UsageLimits, UsageReport, UsageRow, UsageSum } from '../../api/types';
import { Sec } from './Sections';
import { barPercents, costText, limitHot, percentText, resetText, seenText, tokensText } from './usageText';

/** Is the USAGE panel open? The header button sets it. */
export const usagePanel = signal(false);

/** Horizontal bars, one row each: the name, a bar scaled to the largest, then the cost and tokens. */
function Bars({ rows, onPick }: { rows: UsageRow[]; onPick?: (r: UsageRow) => void }) {
  const pct = barPercents(rows);
  return (
    <ul class="ubars">
      {rows.map((r, i) => (
        <li key={r.id ?? r.name}>
          {onPick && r.id ? <button class="uname" onClick={() => onPick(r)} title="Open this session">{r.name || r.id.slice(0, 8)}</button> : <span class="uname">{r.name}</span>}
          <span class="utrack" aria-hidden="true"><i style={`width:${pct[i]}%`} /></span>
          <span class="uval"><b>{costText(r)}</b> <small>{tokensText(r)}</small></span>
        </li>
      ))}
    </ul>
  );
}

function LimitLines({ limits }: { limits: UsageLimits[] }) {
  return (
    <ul class="ulimits">
      {limits.flatMap((l) => l.windows.map((w) => (
        <li key={`${l.agent}-${w.label}`} class={limitHot(w) ? 'hot' : ''}>
          <b>{l.name}</b>: {percentText(w.used_percent)} of the {w.label} limit
          {w.reset ? ' (window has reset since)' : w.resets_at ? `, resets ${resetText(w.resets_at)}` : ''}
          <small> seen {seenText(l.seen_at)}</small>
        </li>
      )))}
    </ul>
  );
}

function Trend({ weeks }: { weeks: UsageReport['weeks'] }) {
  const pct = barPercents(weeks);
  return (
    <ul class="ubars">
      {weeks.map((w, i) => (
        <li key={w.from}>
          <span class="uname">{w.from.slice(5)} to {w.to.slice(5)}</span>
          <span class="utrack" aria-hidden="true"><i style={`width:${pct[i]}%`} /></span>
          <span class="uval"><b>{costText(w)}</b> <small>{tokensText(w)}</small></span>
        </li>
      ))}
    </ul>
  );
}

const WINDOWS = [{ days: 7, label: 'WEEK' }, { days: 30, label: 'MONTH' }];

/** The USAGE panel: limits, this week by project and by agent, the top 10 sessions, four weeks. */
export function UsageHost({ store }: { store: Store }) {
  const open = usagePanel.value;
  const days = useSignal(7);
  const data = useSignal<UsageReport | null>(null);
  const failed = useSignal(false);
  useEffect(() => {
    if (!open) return;
    let live = true;
    data.value = null;
    failed.value = false;
    void store.usage(days.value).then((r) => {
      if (!live) return;
      data.value = r;
      failed.value = r === null;
    });
    return () => { live = false; };
  }, [open, days.value]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopImmediatePropagation();
      usagePanel.value = false;
    };
    addEventListener('keydown', onKey, true);
    return () => removeEventListener('keydown', onKey, true);
  }, [open]);
  if (!open) return null;
  const close = () => { usagePanel.value = false; };
  const r = data.value;
  const empty = !!r && r.totals.total === 0;
  const range = days.value === 7 ? 'THIS WEEK' : 'THIS MONTH';
  const pick = (row: UsageRow) => { close(); store.open(row.id!); };
  return (
    <>
      <div class="scrim" role="presentation" onClick={close} />
      <aside class="pagewrap" aria-label="Usage">
        <div class="page usage">
          <button class="x" onClick={close} aria-label="Close (Esc)">ESC ✕</button>
          <header class="dh"><div><h2>Usage</h2><p class="umeta">{r ? r.estimate : 'API-equivalent estimate, not your bill'}</p></div></header>
          <div class="acts">
            {WINDOWS.map((w) => (
              <button key={w.days} class={`btn sm${days.value === w.days ? '' : ' ghost'}`} aria-pressed={days.value === w.days} onClick={() => { days.value = w.days; }}>{w.label}</button>
            ))}
          </div>
          {!r && !failed.value && <p class="state">Loading…<i class="dots" /></p>}
          {failed.value && <p class="state">This engine cannot report usage. Update eye, then reload.</p>}
          {r && r.limits.length > 0 && <Sec t="PLAN LIMITS"><LimitLines limits={r.limits} /></Sec>}
          {r && empty && (
            <Sec t="NO USAGE YET">
              <p class="uempty">Nothing has been counted for {days.value === 7 ? 'the last week' : 'the last month'}. eye counts what a session uses as it reads it; sessions from before are counted in the background{r.backfill_left > 0 ? ` (${r.backfill_left} to go)` : ''}.</p>
            </Sec>
          )}
          {r && !empty && (
            <>
              <Sec t={`${range} BY PROJECT`} n={costText(r.totals)}><Bars rows={r.by_project} /></Sec>
              <Sec t="BY AGENT"><Bars rows={r.by_agent} /></Sec>
              <Sec t="TOP 10 SESSIONS"><Bars rows={r.top_sessions} onPick={pick} /></Sec>
              <Sec t="LAST 4 WEEKS"><Trend weeks={r.weeks} /></Sec>
              {r.totals.unpriced_tokens > 0 && <p class="unote">{tokensText({ total: r.totals.unpriced_tokens } as UsageSum)} are on models without a price, so their cost is not counted.</p>}
              {r.backfill_left > 0 && <p class="unote">{r.backfill_left} older sessions are still being counted.</p>}
            </>
          )}
        </div>
        <div class="dis" />
      </aside>
    </>
  );
}
