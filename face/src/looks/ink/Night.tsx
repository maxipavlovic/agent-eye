import { useEffect, useRef } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import type { Store } from '../../store';
import type { CardDetail, NightIsolation, NightRun } from '../../api/types';
import { ago } from '../../format';
import { Sec, Fold, openFold } from './Sections';
import { tokensText } from './usageText';

/** Is the Tonight list open? The header chip sets it. */
export const nightPanel = signal(false);

/** The card whose TONIGHT form is open in the panel (an idea card's TONIGHT button sets it). */
export const nightAsk = signal<string | null>(null);

export const ISOLATION_HELP: Record<NightIsolation, string> = {
  worktree: 'A worktree of your repo, on a new branch. Fast and small. A command that builds a path in an unusual way could still add a new file at the top of your repo folder.',
  copy: 'A separate copy of your repo. Your own folder is fully locked: nothing in it can change. Bring the branch into your repo in the morning.',
};

export const LIMIT_HELP = '30% of the weekly plan, 2M or 500k tokens, 90m or 2h of time. One at most; empty: no limit.';

/** `1 h 5 min`, `41 min`, `under 1 min`. */
export function durationText(ms: number | null | undefined): string {
  if (ms == null) return '';
  if (ms < 60_000) return 'under 1 min';
  const m = Math.round(ms / 60_000);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h${m % 60 ? ` ${m % 60} min` : ''}`;
}

/** The words for a run's state, with why when it did not simply finish. */
export function runStatus(r: NightRun): string {
  const base = { planned: 'Planned', starting: 'Starting', lost: 'Lost', running: 'Running', done: 'Done', failed: 'Failed', stopped: 'Stopped', skipped: 'Skipped' }[r.status];
  return r.why && r.status !== 'planned' ? `${base}: ${r.why}` : base;
}

const clock = (ms: number) => new Date(ms).toLocaleString([], { weekday: 'short', hour: '2-digit', minute: '2-digit' });

/** A calm "Tonight: 2" chip while runs are planned (or going). It opens the list. */
export function TonightChip({ store }: { store: Store }) {
  const n = store.night.value;
  if (!n) return null;
  const running = n.open.filter((r) => r.status === 'running').length;
  if (n.planned < 1 && running < 1) return null;
  const text = n.planned > 0 ? `Tonight: ${n.planned}` : `Night: ${running} running`;
  return (
    <button class="btn tonight" aria-haspopup="dialog" aria-pressed={nightPanel.value} title={n.enabled ? `Night shift on, ${n.window}` : 'Night shift is off: open the list to switch it on'} onClick={() => { nightPanel.value = !nightPanel.value; }}>
      {text}
    </button>
  );
}


/** Plan an idea (or continue a session) for tonight: the prompt (a session needs one), one optional limit, the agent and the switch. */
export function NightPlanForm({ store, d, onDone }: { store: Store; d: CardDetail; onDone?: () => void }) {
  const session = d.kind !== 'idea';
  const prompt = useSignal('');
  const limit = useSignal('');
  const agent = useSignal('');
  const sw = useSignal(false);
  const iso = useSignal<NightIsolation | ''>('');
  const busy = useSignal(false);
  const agents = store.agents.value;
  const chosen: NightIsolation = iso.value || store.night.value?.isolation || 'worktree';
  const go = async (e: Event) => {
    e.preventDefault();
    if (busy.value) return;
    busy.value = true;
    const base = { limit: limit.value.trim() || undefined, isolation: chosen };
    const ok = await store.nightPlan(session
      ? { ...base, session_id: d.id, prompt: prompt.value }
      : { ...base, idea_id: d.idea_id!, agent: agent.value || undefined, switch: sw.value });
    busy.value = false;
    if (ok) { prompt.value = ''; onDone?.(); }
  };
  const ref = useRef<HTMLFormElement>(null);
  // opened to be written in: the prompt (a session needs one) takes the focus
  useEffect(() => {
    ref.current?.scrollIntoView({ block: 'nearest' });
    ref.current?.querySelector<HTMLElement>('textarea, input')?.focus({ preventScroll: true });
  }, []);
  const dflt = (a: { id: string; default?: boolean }) => (agent.value || d.agent) === a.id || (!agent.value && !d.agent && !!a.default);
  return (
    <form class="nplan" ref={ref} onSubmit={go} aria-label={session ? 'Continue tonight' : 'Plan for tonight'}>
      {session && (
        <label>WHAT SHOULD IT DO TONIGHT
          <textarea rows={3} value={prompt.value} onInput={(e) => { prompt.value = e.currentTarget.value; }} placeholder="Continue with the failing tests, then stop" />
        </label>
      )}
      <label>LIMIT<span class="hint">{LIMIT_HELP}</span>
        <input value={limit.value} onInput={(e) => { limit.value = e.currentTarget.value; }} placeholder="none" autocomplete="off" spellcheck={false} />
      </label>
      {!session && agents.length > 1 && (
        <fieldset class="pick">
          <legend>AGENT</legend>
          <div class="fchips">
            {agents.map((a) => (
              <button type="button" key={a.id} title={a.name} aria-pressed={dflt(a)} onClick={() => { agent.value = a.id; }}>{a.short}</button>
            ))}
          </div>
        </fieldset>
      )}
      {!session && agents.length > 1 && (
        <fieldset class="pick">
          <legend>WHEN THE PLAN RUNS OUT</legend>
          <div class="fchips">
            <button type="button" aria-pressed={!sw.value} onClick={() => { sw.value = false; }}>Stop</button>
            <button type="button" aria-pressed={sw.value} onClick={() => { sw.value = true; }}>Start again on the other agent</button>
          </div>
        </fieldset>
      )}
      <fieldset class="pick">
        <legend>WORK IN</legend>
        <div class="fchips">
          <button type="button" aria-pressed={chosen === 'worktree'} title={ISOLATION_HELP.worktree} onClick={() => { iso.value = 'worktree'; }}>A worktree of your repo (default)</button>
          <button type="button" aria-pressed={chosen === 'copy'} title={ISOLATION_HELP.copy} onClick={() => { iso.value = 'copy'; }}>A separate copy (your folder fully locked)</button>
        </div>
        <span class="hint">{ISOLATION_HELP[chosen]}</span>
      </fieldset>
      <p class="hint">Runs on a new branch. Nothing is pushed.</p>
      <div class="acts end">
        <button type="submit" class="btn big" disabled={busy.value}>{session ? 'CONTINUE TONIGHT' : 'PLAN FOR TONIGHT'}</button>
      </div>
    </form>
  );
}

/** The TONIGHT button of an idea's panel and its form. */
export function TonightIdea({ d, store }: { d: CardDetail; store: Store }) {
  const open = nightAsk.value === d.id;
  const planned = store.night.value?.open.find((r) => r.idea_id === d.idea_id && r.status !== 'done');
  if (planned) return <p class="fnote" role="status">Planned for tonight{planned.limit ? `, limit ${planned.limit.text}` : ''}. <button class="btn sm ghost" onClick={() => { nightPanel.value = true; }}>OPEN THE LIST</button></p>;
  return (
    <>
      <div class="acts">
        <button class="btn" aria-expanded={open} title="Run it while you sleep, in its own worktree" onClick={() => { nightAsk.value = open ? null : d.id; }}>TONIGHT</button>
      </div>
      {open && <NightPlanForm store={store} d={d} onDone={() => { nightAsk.value = null; }} />}
    </>
  );
}

/** The session whose CONTINUE TONIGHT form is open in the panel (the button, N and the card menu set it). */
export const tonightOpen = signal<string | null>(null);

/** Can this session be continued tonight: a session eye can resume, on an engine with the night shift. */
export function canContinueTonight(d: Pick<CardDetail, 'kind' | 'resumable'>, store: Store): boolean {
  return d.kind !== 'idea' && !!d.resumable && store.night.value !== null;
}

/** The CONTINUE TONIGHT form, in the panel, while it is open. */
export function ContinueTonight({ d, store }: { d: CardDetail; store: Store }) {
  if (!canContinueTonight(d, store) || tonightOpen.value !== d.id) return null;
  return (
    <Sec t="CONTINUE TONIGHT">
      <NightPlanForm store={store} d={d} onDone={() => { tonightOpen.value = null; }} />
    </Sec>
  );
}

/** The question before discarding: what goes, and that uncommitted work would be lost. */
export function discardText(r: NightRun): string {
  const copy = r.isolation === 'copy';
  const what = r.branch ? (copy ? `the copy and its branch ${r.branch}` : `the worktree and the branch ${r.branch}`) : 'the copy of the folder';
  return `Remove ${what}? ${r.branch ? (copy ? 'Commits that are only in the copy are lost unless you brought the branch into your repo. ' : 'Commits that are only on that branch are lost. ') : ''}${r.uncommitted ? 'It has uncommitted changes that will be lost too.' : ''}`.trim();
}

/** NIGHT RUN: how the run went, what it said, what it changed, and what to do with it. */
/** "1 file changed": the first part of a diff stat's last line. */
export function changedText(diff: string | null): string | null {
  const last = diff?.trim().split('\n').pop()?.trim();
  return last ? last.split(',')[0]!.trim() : null;
}

/** The night run in one line at the top of the panel: its status, what it changed and how long it took; REVIEW opens the details. */
export function NightRunLine({ d }: { d: CardDetail }) {
  const r = d.night_run;
  if (!r) return null;
  const parts = [runStatus(r), r.worktree ? changedText(r.diff_stat) : 'discarded', r.duration_ms != null ? durationText(r.duration_ms) : null].filter(Boolean);
  const review = () => {
    openFold('night-run');
    requestAnimationFrame(() => document.getElementById('night-run')?.scrollIntoView({ block: 'start', behavior: 'smooth' }));
  };
  return (
    <p class="nline">
      <b>NIGHT RUN</b><span>{parts.join(' · ')}</span>
      <button class="lnk" onClick={review}>REVIEW</button>
    </p>
  );
}

/** The night run's details, folded in the panel: report, what it changed, where, and RESUME, BRING, OPEN FOLDER, DISCARD. */
export function NightRunSection({ d, store }: { d: CardDetail; store: Store }) {
  const ask = useSignal(false);
  const askBring = useSignal(false);
  const busy = useSignal(false);
  const r = d.night_run;
  if (!r) return null;
  const folder = r.worktree;
  const stoppedByLimit = r.status === 'stopped' && !!r.why;
  const discard = async () => {
    busy.value = true;
    await store.nightDiscard(r.id, !!r.uncommitted);
    busy.value = false;
    ask.value = false;
  };
  const bring = async () => {
    busy.value = true;
    await store.nightBring(r.id);
    busy.value = false;
    askBring.value = false;
  };
  const repoCopy = r.isolation === 'copy' && !!r.branch && !!folder;
  return (
    <Fold name="night-run" t="NIGHT RUN" sum={runStatus(r)} text={r.report ?? undefined}>
      <dl class="nrun">
        <dt>Status</dt><dd>{runStatus(r)}</dd>
        {r.started_at != null && <><dt>Started</dt><dd>{clock(r.started_at)}</dd></>}
        {r.ended_at != null && <><dt>Ended</dt><dd>{clock(r.ended_at)}</dd></>}
        {r.duration_ms != null && <><dt>Took</dt><dd>{durationText(r.duration_ms)}</dd></>}
        {r.tokens != null && <><dt>Tokens</dt><dd>{tokensText({ total: r.tokens } as never)}</dd></>}
        <dt>Worked in</dt><dd>{r.isolation === 'copy' ? 'A separate copy (your folder was fully locked)' : 'A worktree of your repo'}</dd>
        <dt>Limit</dt><dd>{r.limit ? `${r.limit.text}${stoppedByLimit ? ': it stopped the run' : r.status === 'done' ? ': not reached' : ''}` : 'none'}</dd>
      </dl>
      {r.report && <><h4 class="nhead">REPORT</h4><pre class="nreport">{r.report}</pre></>}
      {r.diff_stat && <><h4 class="nhead">WHAT IT CHANGED</h4><pre class="nreport">{r.diff_stat}</pre></>}
      {folder && <p class="nwhere">{r.branch ? <>Branch <b>{r.branch}</b> in </> : <>Copy in </>}<code>{folder}</code></p>}
      {!folder && <p class="dim">Discarded: its worktree and branch are gone.</p>}
      {askBring.value ? (
        <div class="lask" role="alertdialog" aria-label="Bring the branch into your repo">
          <p>Fetch the branch {r.branch} from the copy into your repo? It is added as a new branch; nothing you have checked out changes.</p>
          <button class="btn sm" disabled={busy.value} onClick={() => void bring()}>YES</button>
          <button class="btn sm ghost" disabled={busy.value} onClick={() => { askBring.value = false; }}>CANCEL</button>
        </div>
      ) : ask.value ? (
        <div class="lask" role="alertdialog" aria-label={discardText(r)}>
          <p>{discardText(r)}</p>
          <button class="btn sm" disabled={busy.value} onClick={() => void discard()}>YES</button>
          <button class="btn sm ghost" disabled={busy.value} onClick={() => { ask.value = false; }}>CANCEL</button>
        </div>
      ) : (
        <div class="acts">
          {d.resumable && <button class="btn" title="Open the session in its worktree" onClick={() => void store.resume(d.id)}>RESUME</button>}
          {repoCopy && r.status !== 'running' && <button class="btn" title="Fetch the night branch from the copy into your repo" onClick={() => { askBring.value = true; }}>BRING INTO MY REPO</button>}
          {folder && <button class="btn" onClick={() => void store.nightOpen(r.id)}>OPEN FOLDER</button>}
          {folder && r.status !== 'running' && <button class="btn ghost" onClick={() => { ask.value = true; }}>DISCARD</button>}
        </div>
      )}
    </Fold>
  );
}

function RunRow({ r, store }: { r: NightRun; store: Store }) {
  const edit = useSignal(false);
  const text = useSignal(r.limit?.text.replace(/ minutes$/, 'm').replace(/^weekly plan above /, '') ?? '');
  const planned = r.status === 'planned';
  const save = async (e: Event) => {
    e.preventDefault();
    if (await store.nightLimit(r.id, text.value)) edit.value = false;
  };
  return (
    <li class={`trow ${r.status}`}>
      <div class="nt">
        <b>{r.kind === 'idea' ? 'Idea' : 'Continue'}</b> <span class="na">{r.agent}</span>
        <span class="np">{r.prompt.length > 110 ? `${r.prompt.slice(0, 110)}…` : r.prompt}</span>
      </div>
      <div class="ns">{runStatus(r)}{r.limit ? ` · limit ${r.limit.text}` : ''}{planned ? ` · night of ${r.night}` : ''}{r.started_at && !planned ? ` · started ${ago(r.started_at)}` : ''}</div>
      {edit.value ? (
        <form class="nlim" onSubmit={save}>
          <input value={text.value} onInput={(e) => { text.value = e.currentTarget.value; }} placeholder="none" aria-label="Limit" autocomplete="off" spellcheck={false} title={LIMIT_HELP} />
          <button class="btn sm" type="submit">SAVE</button>
          <button class="btn sm ghost" type="button" onClick={() => { edit.value = false; }}>CANCEL</button>
        </form>
      ) : (
        <div class="acts">
          {planned && <button class="btn sm" onClick={() => { edit.value = true; }}>CHANGE LIMIT</button>}
          {planned && <button class="btn sm ghost" onClick={() => void store.nightCancel(r.id)}>CANCEL RUN</button>}
          {r.run_session_id && <button class="btn sm" onClick={() => { nightPanel.value = false; store.open(r.run_session_id!); }}>OPEN</button>}
        </div>
      )}
    </li>
  );
}

/** The Tonight list: the switch, the window, how many at once, and the runs. */
export function NightHost({ store }: { store: Store }) {
  const open = nightPanel.value;
  const win = useSignal('');
  useEffect(() => { if (open) { store.loadNight(); win.value = store.night.value?.window ?? ''; } }, [open]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopImmediatePropagation();
      nightPanel.value = false;
    };
    addEventListener('keydown', onKey, true);
    return () => removeEventListener('keydown', onKey, true);
  }, [open]);
  const n = store.night.value;
  if (!open || !n) return null;
  const close = () => { nightPanel.value = false; };
  const saveWindow = async (e: Event) => { e.preventDefault(); await store.nightSettings({ window: win.value }); };
  return (
    <>
      <div class="scrim" role="presentation" onClick={close} />
      <aside class="pagewrap" aria-label="Tonight">
        <div class="page night">
          <button class="x" onClick={close} aria-label="Close (Esc)">ESC ✕</button>
          <header class="dh"><div><h2>Tonight</h2><p class="umeta">Planned runs start by themselves inside the window, when nothing you work in is live.</p></div></header>
          <div class="acts">
            <button class={`btn${n.enabled ? '' : ' ghost'}`} aria-pressed={n.enabled} onClick={() => void store.nightSettings({ enabled: !n.enabled })}>{n.enabled ? 'NIGHT SHIFT ON' : 'NIGHT SHIFT OFF'}</button>
            <span class="umeta">{n.enabled ? (n.in_window ? 'The window is open now.' : 'Waiting for the window.') : 'Nothing starts until you switch it on.'}</span>
          </div>
          <Sec t="WINDOW AND PARALLEL">
            <form class="nset" onSubmit={saveWindow}>
              <label>WINDOW (LOCAL TIME)<input value={win.value} onInput={(e) => { win.value = e.currentTarget.value; }} placeholder="01:00-07:00" autocomplete="off" spellcheck={false} /></label>
              <button class="btn sm" type="submit">SET WINDOW</button>
            </form>
            <div class="nset">
              <label>AT ONCE
                <select value={String(n.parallel)} onChange={(e) => void store.nightSettings({ parallel: Number(e.currentTarget.value) })}>
                  {[1, 2, 3, 4].map((k) => <option key={k} value={String(k)}>{k}</option>)}
                </select>
              </label>
              <span class="umeta">Quiet for {n.quiet_minutes} minutes before a run starts.</span>
            </div>
          </Sec>
          <Sec t="PLANNED AND RUNNING" n={n.open.length}>
            {n.open.length === 0 ? <p class="dim">Nothing planned. Use TONIGHT on an idea, or CONTINUE TONIGHT on a session.</p> : <ul class="nlist">{n.open.map((r) => <RunRow key={r.id} r={r} store={store} />)}</ul>}
          </Sec>
          {n.recent.length > 0 && <Sec t="RECENT" n={n.recent.length}><ul class="nlist">{n.recent.slice(0, 8).map((r) => <RunRow key={r.id} r={r} store={store} />)}</ul></Sec>}
        </div>
        <div class="dis" />
      </aside>
    </>
  );
}
