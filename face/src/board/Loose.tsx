import { signal, useSignal } from '@preact/signals';
import type { Store } from '../store';
import type { CardDetail, LooseAct, LooseEnd } from '../api/types';
import { copy } from './copy';

/** What is waiting for a yes: the exact words of the question, and what a yes does. `key` is the item's, or `*` for all. */
interface Ask { key: string; text: string; run: () => Promise<unknown> }

/** The session whose list of leftovers is open (SHOW, and L, open it). */
export const leftoversOpen = signal<string | null>(null);

/** The question before ignoring an item. */
export const ignoreText = (it: LooseEnd) => `Stop listing ${it.what}?`;

/** The question before ignoring all of a session's leftovers. */
export const ignoreAllText = (n: number) => `Stop listing ${n === 1 ? 'this leftover' : `all ${n} leftovers`} of this session?`;

/** The question before sending the session back to its agent. */
export const sendText = (agent: string, n: number) =>
  `Open ${agent} in this session with a message that lists ${n === 1 ? 'the thing' : `the ${n} things`} it left behind?`;

const NOUN: Record<LooseEnd['kind'], [string, string]> = {
  process: ['process', 'processes'],
  container: ['container', 'containers'],
  worktree: ['worktree', 'worktrees'],
  unpushed: ['unpushed commit', 'unpushed commits'],
  uncommitted: ['set of changed files', 'sets of changed files'],
};

/** "2 processes and 1 container": the items of one action, by kind. */
function byKind(items: LooseEnd[]): string {
  const n = new Map<LooseEnd['kind'], number>();
  for (const it of items) n.set(it.kind, (n.get(it.kind) ?? 0) + 1);
  return [...n].map(([k, c]) => `${c} ${NOUN[k][c === 1 ? 0 : 1]}`).join(' and ');
}

/** The question before REMOVE ALL: what it stops, what it removes, and what stays because eye never pushes or commits. */
export function removeAllText(items: LooseEnd[]): string {
  const stop = items.filter((it) => it.action === 'stop');
  const remove = items.filter((it) => it.action === 'remove');
  const stays = items.filter((it) => it.action !== 'stop' && it.action !== 'remove');
  const does = [stop.length ? `stop ${byKind(stop)}` : '', remove.length ? `remove ${byKind(remove)}` : ''].filter(Boolean).join(', and ');
  const q = `${does.charAt(0).toUpperCase()}${does.slice(1)}?`;
  return stays.length ? `${q} ${byKind(stays)} ${stays.length === 1 ? 'stays' : 'stay'} listed: eye never pushes or commits for you.` : q;
}

/**
 * LEFTOVERS: what the session left running, in git or in a scratchpad, in two lines: how many and what, and
 * REMOVE ALL… (what eye can stop or remove) and IGNORE ALL. SHOW opens the list, one line per item with its own
 * actions (stop, remove, or copy the command: eye never pushes or commits for you), and SEND BACK TO THE AGENT.
 * Every action that changes something asks first, right here in the page, with the words of what will happen.
 */
export function Leftovers({ d, store }: { d: CardDetail; store: Store }) {
  const ask = useSignal<Ask | null>(null);
  const busy = useSignal(false);
  const items = d.loose_ends ?? [];
  if (d.kind === 'idea' || items.length === 0) return null;
  const open = leftoversOpen.value === d.id;
  const removable = items.filter((it) => it.action === 'stop' || it.action === 'remove');
  const go = (a: Ask) => { ask.value = a; };
  const yes = async () => {
    const a = ask.value;
    if (!a || busy.value) return;
    busy.value = true;
    try { await a.run(); } finally { busy.value = false; ask.value = null; }
  };
  const act = (it: LooseEnd, what: LooseAct, text: string) => go({ key: it.key, text, run: () => store.looseAct(what, it.key) });
  const Question = ({ a }: { a: Ask }) => (
    <div class="lask" role="alertdialog" aria-label={a.text}>
      <p>{a.text}</p>
      <button class="btn sm" disabled={busy.value} onClick={() => void yes()}>YES</button>
      <button class="btn sm ghost" disabled={busy.value} onClick={() => { ask.value = null; }}>CANCEL</button>
    </div>
  );
  return (
    <section class="sec lefto" id="leftovers">
      <h4>LEFTOVERS<b>{items.length}</b></h4>
      <p class="lsum">{d.loose_summary ?? items.map((it) => it.line).join(', ')}</p>
      {ask.value?.key === '*' ? <Question a={ask.value} /> : (
        <div class="lacts">
          <button
            class="btn sm"
            disabled={!removable.length}
            title={removable.length ? 'Stop or remove what eye can; it asks first' : 'Nothing here can be removed by eye: copy the commands'}
            onClick={() => go({ key: '*', text: removeAllText(items), run: () => store.looseActAll(removable.map((it) => ({ action: it.action as LooseAct, key: it.key }))) })}
          >REMOVE ALL…</button>
          <button class="btn sm ghost" onClick={() => go({ key: '*', text: ignoreAllText(items.length), run: () => store.looseActAll(items.map((it) => ({ action: 'ignore' as const, key: it.key }))) })}>IGNORE ALL</button>
          <button class="lnk" aria-expanded={open} onClick={() => { leftoversOpen.value = open ? null : d.id; }}>{open ? 'HIDE' : 'SHOW'}</button>
        </div>
      )}
      {open && (
        <>
          <ul class="dloose">
            {items.map((it) => (
              <li key={it.key} class={`k-${it.kind}`}>
                <span class="ll">{it.line}</span>
                {ask.value?.key === it.key ? <Question a={ask.value} /> : (
                  <span class="la">
                    {it.action === 'stop' && <button class="btn sm" onClick={() => act(it, 'stop', it.confirm ?? `Stop ${it.what}?`)}>STOP</button>}
                    {it.action === 'remove' && <button class="btn sm" onClick={() => act(it, 'remove', it.confirm ?? `Remove ${it.what}?`)}>REMOVE</button>}
                    {it.command && <button class="btn sm" title={it.command} onClick={() => copy(it.command!, store)}>COPY COMMAND</button>}
                    <button class="btn sm ghost" onClick={() => act(it, 'ignore', ignoreText(it))}>IGNORE</button>
                  </span>
                )}
              </li>
            ))}
          </ul>
          {ask.value?.key === 'send' ? <Question a={ask.value} /> : (
            <div class="acts">
              <button class="btn" title="Resume the session with a message that lists these" onClick={() => go({ key: 'send', text: sendText(d.agent_name, items.length), run: () => store.looseSend(d.id) })}>SEND BACK TO THE AGENT</button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
