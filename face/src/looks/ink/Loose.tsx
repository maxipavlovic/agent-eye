import { useSignal } from '@preact/signals';
import type { Store } from '../../store';
import type { CardDetail, LooseAct, LooseEnd } from '../../api/types';
import { copy } from './copy';
import { Sec } from './Sections';

/** What is waiting for a yes: the exact words of the question, and what a yes does. */
interface Ask { key: string | null; text: string; run: () => Promise<unknown> }

/** The question before ignoring an item. */
export const ignoreText = (it: LooseEnd) => `Stop listing ${it.what}?`;

/** The question before sending the session back to its agent. */
export const sendText = (agent: string, n: number) =>
  `Open ${agent} in this session with a message that lists ${n === 1 ? 'the thing' : `the ${n} things`} it left behind?`;

/**
 * LEFT BEHIND: what the session left running, in git or in a scratchpad. One key per item: stop,
 * remove, or copy the command (eye never pushes or commits for you), ignore. Every action that
 * changes something asks first, right here in the page, with the words of what will happen.
 */
export function LeftBehind({ d, store }: { d: CardDetail; store: Store }) {
  const ask = useSignal<Ask | null>(null);
  const busy = useSignal(false);
  const items = d.loose_ends ?? [];
  if (d.kind === 'idea' || items.length === 0) return null;
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
    <Sec t="LEFT BEHIND" n={items.length}>
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
      {ask.value?.key === null ? <Question a={ask.value} /> : (
        <div class="acts">
          <button class="btn" title="Resume the session with a message that lists these" onClick={() => go({ key: null, text: sendText(d.agent_name, items.length), run: () => store.looseSend(d.id) })}>SEND BACK TO THE AGENT</button>
        </div>
      )}
    </Sec>
  );
}
