import { useEffect } from 'preact/hooks';
import { useSignal } from '@preact/signals';
import type { Store } from '../../store';
import type { Message } from '../../api/types';
import { codeParts, msgTime } from '../../format';
import { Hl } from './highlight';
import { FoldSec, Sec } from './Sections';

/** Message text with `inline code` spans; the search words are marked inside both. */
function Text({ s }: { s: string }) {
  return <>{codeParts(s).map((p, i) => (p.code ? <code key={i}><Hl s={p.text} /></code> : <Hl key={i} s={p.text} />))}</>;
}

const PAGE = 10;

/**
 * The newest messages of the session, loaded when the panel opens; older ones on request.
 * `agent` is the short name of the session's agent, shown above its replies.
 */
export function Conversation({ store, id, agent, fallback }: { store: Store; id: string; agent: string; fallback: string[] }) {
  const msgs = useSignal<Message[]>([]);
  const next = useSignal<number | null>(null);
  const state = useSignal<'loading' | 'ready' | 'failed'>('loading');
  const older = useSignal(false);
  const open = useSignal(false);
  useEffect(() => { open.value = false; }, [id]);
  useEffect(() => {
    if (!open.value) return;
    let live = true;
    msgs.value = []; next.value = null; state.value = 'loading';
    void store.conversation(id, null, PAGE).then((p) => {
      if (!live) return;
      if (!p) { state.value = 'failed'; return; }
      msgs.value = p.messages; next.value = p.more ? p.next_before : null; state.value = 'ready';
    });
    return () => { live = false; };
  }, [id, open.value]);
  const more = async () => {
    const before = next.value;
    if (before === null || older.value) return;
    older.value = true;
    const p = await store.conversation(id, before, PAGE);
    older.value = false;
    if (!p || store.selected.value !== id) return;
    msgs.value = [...p.messages, ...msgs.value];
    next.value = p.more ? p.next_before : null;
  };
  // An engine without the conversation endpoint: keep showing the stored prompts.
  if (state.value === 'failed') {
    return fallback.length ? <Sec t="LAST PROMPTS">{fallback.map((p, j) => <blockquote class="prompt" key={j}><Hl s={p} /></blockquote>)}</Sec> : null;
  }
  return (
    <FoldSec t="CONVERSATION" open={open.value} toggle={() => { open.value = !open.value; }}>
      {state.value === 'ready' && msgs.value.length === 0 && <p class="dim">No messages to show.</p>}
      {state.value === 'loading' && <p class="dim">Loading the conversation<i class="dots" /></p>}
      {next.value !== null && (
        <p class="earlier"><button class="lnk" disabled={older.value} onClick={() => void more()}>{older.value ? 'LOADING…' : 'SHOW EARLIER'}</button></p>
      )}
      <ol class="conv">
        {msgs.value.map((m, i) => (
          <li key={`${next.value}-${i}-${m.ts}`} class={`msg ${m.role === 'user' ? 'you' : 'agent'}`}>
            <p class="who"><b>{m.role === 'user' ? 'You' : agent}</b>{m.ts && <time>{msgTime(m.ts)}</time>}</p>
            <p class="mt"><Text s={m.text} />{m.truncated && <span class="cut"> (cut)</span>}</p>
            {m.role === 'assistant' && m.tools > 0 && <p class="tools">ran {m.tools} tool{m.tools === 1 ? '' : 's'}</p>}
          </li>
        ))}
      </ol>
    </FoldSec>
  );
}
