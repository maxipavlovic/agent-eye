import type { ComponentChildren } from 'preact';
import { useSignal } from '@preact/signals';
import { Pix } from '../../pixel/Pix';
import type { CardDetail, Output, OutputKind, Stream, Tokens, Topic } from '../../api/types';
import { folderGroup, OUTPUT_NAMES, outputLabel, tok } from '../../format';
import { INK, MARKS, OUT_TAG } from './art';
import { Hl } from './highlight';
import { Mark } from './pieces';

/** A titled block of the detail panel, with an optional count. */
export function Sec({ t, n, children }: { t: string; n?: number | string; children: ComponentChildren }) {
  return (
    <section class="sec">
      <h4>{t}{n !== undefined && <b>{n}</b>}</h4>
      {children}
    </section>
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

/** Sub-agents: all done = one line with a toggle; otherwise running ones first, done ones behind "+N done". */
export function Kids({ kids, total }: { kids: Stream[]; total: number }) {
  const open = useSignal(false);
  const running = kids.filter((k) => k.status !== 'done');
  const done = kids.filter((k) => k.status === 'done');
  const hidden = Math.max(0, total - kids.length);
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
        {open.value && <ul class="kids">{done.map(row)}{hidden > 0 && <li class="dim">+{hidden} more</li>}</ul>}
      </>
    );
  }
  const doneN = done.length + hidden;
  return (
    <>
      <ul class="kids">{running.map(row)}{open.value && done.map(row)}</ul>
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
      <ul class={g.base ? 'ind' : ''}>
        {g.rels.slice(0, 12).map((r, j) => (
          <li key={j}><span class="fpath">{r}</span></li>
        ))}
        {g.rels.length > 12 && <li class="dim">+{g.rels.length - 12} more</li>}
      </ul>
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
            <ul>
              {items.slice(0, 8).map((o, j) => (
                <li key={j}>
                  <span class="ot">{OUT_TAG[k]}</span>
                  {o.url ? <a href={o.url} target="_blank" rel="noopener noreferrer">{outputLabel(o)}</a> : <span title={o.value}>{outputLabel(o)}</span>}
                </li>
              ))}
              {items.length > 8 && <li class="dim">+{items.length - 8} more</li>}
            </ul>
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
      </dl>
    </Sec>
  );
}
