import { useEffect, useRef } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import type { Store } from '../../store';

/** The session the FORGET… confirm is open for, or null. */
export const forgetFor = signal<{ id: string; title: string; agent: string } | null>(null);

let opener: HTMLElement | null = null;

/** Open the confirm for a session, remembering what had focus so closing can give it back. */
export function openForget(c: { id: string; title: string; agent_name: string }) {
  opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  forgetFor.value = { id: c.id, title: c.title, agent: c.agent_name };
}

export function closeForget() {
  forgetFor.value = null;
  const back = opener;
  opener = null;
  if (back?.isConnected) back.focus();
}

/** Exactly what goes: eye's copy always, and the agent's own transcript only when the box is ticked. */
export function forgetText(agent: string, transcript: boolean): string {
  return transcript
    ? `Remove this session from eye, and its transcript from ${agent}? This cannot be undone.`
    : 'Remove this session from eye? This cannot be undone.';
}

function ForgetConfirm({ store, id, title, agent }: { store: Store; id: string; title: string; agent: string }) {
  const transcript = useSignal(false);
  const busy = useSignal(false);
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => { cancel.current?.focus(); }, []);
  const go = async (e: Event) => {
    e.preventDefault();
    if (busy.value) return;
    busy.value = true;
    await store.forget(id, transcript.value);
    busy.value = false;
    forgetFor.value = null;
    opener = null;
  };
  return (
    <>
      <div class="scrim" role="presentation" onClick={closeForget} />
      <form class="modal forget" aria-label="Forget this session" aria-describedby="forget-text" onSubmit={go}>
        <div class="dis" />
        <h2>FORGET THIS SESSION</h2>
        <p class="fname">{title}</p>
        <p id="forget-text" class="ftext">{forgetText(agent, transcript.value)}</p>
        <label class="fcheck">
          <input type="checkbox" checked={transcript.value} onChange={(e) => { transcript.value = e.currentTarget.checked; }} />
          <span>Also remove its transcript from {agent}</span>
        </label>
        {!transcript.value && <p class="fnote">{agent} keeps its own copy, so eye finds the session again at its next scan.</p>}
        <div class="acts end">
          <button type="button" class="btn ghost" ref={cancel} onClick={closeForget}>CANCEL</button>
          <button type="submit" class="btn big danger" disabled={busy.value}>FORGET</button>
        </div>
      </form>
    </>
  );
}

/** The confirm when asked for. */
export function ForgetHost({ store }: { store: Store }) {
  const f = forgetFor.value;
  if (!f) return null;
  return <ForgetConfirm key={f.id} store={store} {...f} />;
}
