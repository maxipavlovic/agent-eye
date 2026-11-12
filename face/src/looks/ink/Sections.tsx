import type { ComponentChildren } from 'preact';
import { useSignal } from '@preact/signals';
import { Pix } from '../../pixel/Pix';
import type { CardDetail, Output, OutputKind, Stream, Tokens, Topic } from '../../api/types';
import { folderGroup, OUTPUT_NAMES, outputLabel, tok } from '../../format';
import { INK, MARKS, OUT_TAG } from './art';
import { Hl } from './highlight';
import { Mark } from './pieces';
import { WorkedRows } from './WorkedRows';

/** A titled block of the detail panel, with an optional count. */
export function Sec({ t, n, children }: { t: string; n?: number | string; children: ComponentChildren }) {
  return (
    <section class="sec">
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

/** The session's Remote Control link, when it is on. */
export function Remote({ d }: { d: CardDetail }) {
  if (!d.remote_url) return null;
  return (
    <p class="remote"><span class="k">REMOTE CONTROL</span> on · <a href={d.remote_url} target="_blank" rel="noopener">open in browser</a></p>
  );
}

/** A section that starts folded: the heading (with an optional one-line summary) opens it. */
export function FoldSec({ t, sum, open, toggle, children }: { t: string; sum?: string; open: boolean; toggle: () => void; children: ComponentChildren }) {
  return (
    <section class={`sec fsec${open ? ' open' : ''}`}>
      <h4>
        <button class="fhead" onClick={toggle} aria-expanded={open}>
          {t}{sum && <span class="fsum">{sum}</span>}<span class="frule" /><span class="fact">{open ? 'HIDE' : 'SHOW'}</span>
        </button>
      </h4>
      {open && children}
    </section>
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
  const running = kids.filter((k) => k.status !== 'done');
  const done = kids.filter((k) => k.status === 'done');
  // children the session counts but has no line for
  const hidden = Math.max(0, total - kids.length);
  const unlisted = hidden > 0 && <p class="dim">and {hidden} more, not listed</p>;
  const row = (k: Stream) => (
    <li key={k.key}><Pix rows={MARKS[k.status === 'done' ? 'done' : 'active']} pal={INK} scale={2} /><span>{k.title}</span></li>
  );
  if (running.length === 0) {
    return (
      <>
        <p class="kidsum">
          <Pix rows={MARKS.done} pal={INK} scale={2} />
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
  const open = useSignal(false);
  return (
    <FoldSec t="TOKENS" sum={tok(t.total)} open={open.value} toggle={() => { open.value = !open.value; }}>
      <dl class="where toks">
        <dt>TOTAL</dt><dd><b>{n(t.total)}</b> <span class="dim">{tok(t.total)}</span></dd>
        <dt>INPUT</dt><dd>{n(t.input)}</dd>
        <dt>OUTPUT</dt><dd>{n(t.output)}</dd>
        <dt>CACHE WRITE</dt><dd>{n(t.cache_write)}</dd>
        <dt>CACHE READ</dt><dd>{n(t.cache_read)}</dd>
        {t.subagents_total > 0 && (<><dt>SUB-AGENTS</dt><dd>{n(t.subagents_total)} <span class="dim">({Math.round((t.subagents_total / Math.max(1, t.total)) * 100)}% of the total)</span></dd></>)}
      </dl>
    </FoldSec>
  );
}

export function Where({ d }: { d: CardDetail }) {
  return (
    <Sec t="WHERE">
      <dl class="where">
        {d.folder && (<><dt>FOLDER</dt><dd>{d.folder}</dd></>)}
        {d.branch && (<><dt>BRANCH</dt><dd>{d.branch}</dd></>)}
        {d.started_at && (<><dt>STARTED</dt><dd>{new Date(d.started_at).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })}</dd></>)}
        <WorkedRows w={d.worked} />
      </dl>
    </Sec>
  );
}
