import type { ComponentChildren } from 'preact';
import { useContext } from 'preact/hooks';
import { signal, useSignal, type Signal } from '@preact/signals';
import { pushPref, type PrefKey } from '../prefs';
import type { CardDetail, Output, OutputKind, Stream, Tokens, Topic } from '../api/types';
import { folderGroup, OUTPUT_NAMES, outputLabel, tok } from '../format';
import { useArt, useLook } from './look';
import { Mark, OUT_TAG } from './pieces';
import { Hl, HlWords } from './highlight';
import { costText, tokensText } from './usageText';
import { WorkedRows } from './WorkedRows';

/** A titled block of the detail panel, with an optional count. */
export function Sec({ t, n, id, children }: { t: string; n?: number | string; id?: string; children: ComponentChildren }) {
  return (
    <section class="sec" id={id}>
      <h4>{t}{n !== undefined && <b>{n}</b>}</h4>
      {children}
    </section>
  );
}

/** What an open session does on its own: each background task and loop on a line. */
export function Running({ d }: { d: CardDetail }) {
  const tasks = d.background ?? [];
  const loops = d.loops ?? [];
  if (tasks.length + loops.length === 0) return null;
  return (
    <Sec t="RUNNING" n={tasks.length + loops.length}>
      <ul class="druns">
        {tasks.map((t) => <li key={t.id}><span class="k">{t.kind.toUpperCase()}</span>{t.line}</li>)}
        {loops.map((l, i) => <li key={i}><span class="k">LOOP</span>{l.line}{l.prompt && l.kind !== 'goal' && <small>{l.prompt}</small>}</li>)}
      </ul>
    </Sec>
  );
}

/** A section that starts folded: the heading (with an optional one-line summary) opens it. */
export function FoldSec({ t, sum, open, toggle, id, children }: { t: string; sum?: string; open: boolean; toggle: () => void; id?: string; children: ComponentChildren }) {
  return (
    <section class={`sec fsec${open ? ' open' : ''}`} id={id}>
      <h4>
        <button class="fhead" onClick={toggle} aria-expanded={open}>
          {t}{sum && <span class="fsum">{sum}</span>}<span class="frule" /><span class="fact">{open ? 'HIDE' : 'SHOW'}</span>
        </button>
      </h4>
      {open && children}
    </section>
  );
}

/** The folded sections of the panel kept open, by name, per look, in this browser and the engine (`eye.<look>.panel`). */
const kept = new Map<string, Signal<Set<string>>>();
const panelKey = (look: string): PrefKey => `eye.${look}.panel`;

function keptFor(look: string): Signal<Set<string>> {
  let open = kept.get(look);
  if (!open) {
    let saved = '';
    try { saved = localStorage.getItem(panelKey(look)) ?? ''; } catch { /* storage blocked */ }
    open = signal(new Set(saved.split(',').filter(Boolean)));
    kept.set(look, open);
  }
  return open;
}

function keep(look: string, name: string, open: boolean) {
  const sections = keptFor(look);
  const next = new Set(sections.value);
  if (open) next.add(name); else next.delete(name);
  sections.value = next;
  pushPref(panelKey(look), [...next].join(','));
}

/** Read the kept sections again from storage (a test starts from a clean page). */
export function resetFolds() {
  kept.clear();
}

/** Open a folded section of the panel in `look` (REVIEW on the night run's line does). */
export function openFold(look: string, name: string) {
  if (!keptFor(look).value.has(name)) keep(look, name, true);
}

/** Whether the folded section `name` is open in the board's look, and the toggle that remembers it. */
export function useFold(name: string): [boolean, () => void] {
  const look = useLook().name;
  const open = keptFor(look).value.has(name);
  return [open, () => keep(look, name, !open)];
}

/**
 * A section of the panel's details: one line with SHOW until opened, and eye remembers which ones you keep
 * open. Opened from a search, a section whose `text` holds a search word opens by itself (HIDE folds it).
 */
export function Fold({ name, t, sum, text, children }: { name: string; t: string; sum?: string; text?: string; children: ComponentChildren }) {
  const re = useContext(HlWords);
  const shut = useSignal(false);
  const [open, toggle] = useFold(name);
  const hit = !!re && !!text && new RegExp(re.source, 'i').test(text) && !shut.value;
  const shown = open || hit;
  return (
    <FoldSec t={t} sum={sum} open={shown} toggle={() => { if (shown && !open) shut.value = true; else toggle(); }} id={name}>
      {children}
    </FoldSec>
  );
}

/** The first `n` items, and a `+N MORE` button that shows the rest in place; SHOW LESS folds them again. */
export function MoreList<T>({ items, n, row, cls }: { items: T[]; n: number; row: (item: T, i: number) => ComponentChildren; cls?: string }) {
  const open = useSignal(false);
  return (
    <ul class={cls}>
      {(open.value ? items : items.slice(0, n)).map((item, i) => row(item, i))}
      {items.length > n && (
        <li class="more">
          <button class="lnk" aria-expanded={open.value} onClick={() => { open.value = !open.value; }}>{open.value ? 'SHOW LESS' : `+${items.length - n} MORE`}</button>
        </li>
      )}
    </ul>
  );
}

export function Topics({ topics }: { topics: Topic[] }) {
  return (
    <Sec t="TOPICS" n={topics.length}>
      <ul class="dtopics">
        {topics.map((t) => (
          <li key={t.key}>
            <Mark s={t.status} scale={2} />
            <div><b><Hl s={t.title} /></b>{t.next && <span><Hl s={t.next} /></span>}</div>
          </li>
        ))}
      </ul>
    </Sec>
  );
}

const KIDS_SHOWN = 8;

/** Sub-agents: all done = one line with a toggle; otherwise running ones first, done ones behind "+N done". Long lists show 8 and +N MORE. */
export function Kids({ kids, total }: { kids: Stream[]; total: number }) {
  const open = useSignal(false);
  const art = useArt();
  const running = kids.filter((k) => k.status !== 'done');
  const done = kids.filter((k) => k.status === 'done');
  // children the session counts but has no line for
  const hidden = Math.max(0, total - kids.length);
  const unlisted = hidden > 0 && <p class="dim">and {hidden} more, not listed</p>;
  const row = (k: Stream) => (
    <li key={k.key}>{art.mark(k.status === 'done' ? 'done' : 'active', 2)}<span>{k.title}</span></li>
  );
  if (running.length === 0) {
    return (
      <>
        <p class="kidsum">
          {art.mark('done', 2)}
          <span>{total} sub-agent{total === 1 ? '' : 's'}, all done</span>
          <button class="lnk" aria-expanded={open.value} onClick={() => (open.value = !open.value)}>{open.value ? 'HIDE' : 'SHOW'}</button>
        </p>
        {open.value && <><MoreList items={done} n={KIDS_SHOWN} row={row} cls="kids" />{unlisted}</>}
      </>
    );
  }
  const doneN = done.length + hidden;
  return (
    <>
      <MoreList items={running} n={KIDS_SHOWN} row={row} cls="kids" />
      {open.value && <><MoreList items={done} n={KIDS_SHOWN} row={row} cls="kids" />{unlisted}</>}
      {doneN > 0 && (
        <p class="dim"><button class="lnk" aria-expanded={open.value} onClick={() => (open.value = !open.value)}>{open.value ? 'HIDE DONE' : `+${doneN} DONE`}</button></p>
      )}
    </>
  );
}

/** Folders relative to home; a shared parent is printed once. */
function Folders({ items }: { items: Output[] }) {
  const g = folderGroup(items.map((o) => o.value));
  return (
    <>
      {g.base && <p class="fbase">{g.base}</p>}
      <MoreList items={g.rels} n={12} cls={g.base ? 'ind' : ''} row={(r, j) => <li key={j}><span class="fpath">{r}</span></li>} />
    </>
  );
}

function groupOutputs(os: Output[]) {
  const by = new Map<OutputKind, Output[]>();
  for (const o of os) by.set(o.kind, [...(by.get(o.kind) ?? []), o]);
  return (['pr', 'artifact', 'jira', 'commit', 'folder'] as OutputKind[]).filter((k) => by.has(k)).map((k) => ({ k, items: by.get(k)! }));
}

/** What the session produced, grouped by kind: pull requests, artifacts, Jira keys, commits, folders. */
export function Outputs({ outputs }: { outputs: Output[] }) {
  const groups = groupOutputs(outputs);
  if (!groups.length) return null;
  return (
    <Sec t="OUTPUTS" n={outputs.length}>
      {groups.map(({ k, items }) => (
        <div class="og" key={k}>
          <h5>{OUTPUT_NAMES[k]} <b>{items.length}</b></h5>
          {k === 'folder' ? <Folders items={items} /> : (
            <MoreList items={items} n={8} row={(o, j) => (
              <li key={j}>
                <span class="ot">{OUT_TAG[k]}</span>
                {o.url ? <a href={o.url} target="_blank" rel="noopener noreferrer">{outputLabel(o)}</a> : <span title={o.value}>{outputLabel(o)}</span>}
              </li>
            )} />
          )}
        </div>
      ))}
    </Sec>
  );
}

export function TokenSec({ t }: { t: Tokens }) {
  const n = (v: number) => v.toLocaleString('en-GB');
  return (
    <Fold name="tokens" t="TOKENS" sum={tok(t.total)}>
      <dl class="where toks">
        <dt>TOTAL</dt><dd><b>{n(t.total)}</b> <span class="dim">{tok(t.total)}</span></dd>
        <dt>INPUT</dt><dd>{n(t.input)}</dd>
        <dt>OUTPUT</dt><dd>{n(t.output)}</dd>
        <dt>CACHE WRITE</dt><dd>{n(t.cache_write)}</dd>
        <dt>CACHE READ</dt><dd>{n(t.cache_read)}</dd>
        {t.subagents_total > 0 && (<><dt>SUB-AGENTS</dt><dd>{n(t.subagents_total)} <span class="dim">({Math.round((t.subagents_total / Math.max(1, t.total)) * 100)}% of the total)</span></dd></>)}
      </dl>
    </Fold>
  );
}

export function Where({ d }: { d: CardDetail }) {
  const u = d.usage && d.usage.total > 0 ? d.usage : null;
  return (
    <Sec t="WHERE">
      <dl class="where">
        {d.folder && (<><dt>FOLDER</dt><dd>{d.folder}</dd></>)}
        {d.branch && (<><dt>BRANCH</dt><dd>{d.branch}</dd></>)}
        {d.started_at && (<><dt>STARTED</dt><dd>{new Date(d.started_at).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })}</dd></>)}
        <WorkedRows w={d.worked} />
        {u && (<><dt>COST</dt><dd>{costText(u)} · {tokensText(u)}</dd></>)}
      </dl>
      {u && <p class="rhint">{u.estimate}</p>}
    </Sec>
  );
}
