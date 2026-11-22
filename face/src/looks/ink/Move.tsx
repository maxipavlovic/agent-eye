import type { Store } from '../../store';
import type { CardDetail, MoveTarget } from '../../api/types';

/** The quick-move targets; the underlined letter of each label is its key (N, W, P, D). */
export const QUICK: Array<{ to: MoveTarget; label: string; key: string }> = [
  { to: 'needs_you', label: 'NEEDS YOU', key: 'n' },
  { to: 'waiting', label: 'WAITING', key: 'w' },
  { to: 'paused', label: 'PAUSED', key: 'p' },
  { to: 'done', label: 'DONE', key: 'd' },
];

/** A label with the letter of its key underlined, the way RESUME shows R. */
export function Keyed({ label, k }: { label: string; k: string }) {
  const i = label.toLowerCase().indexOf(k.toLowerCase());
  return i < 0 ? <>{label}</> : <>{label.slice(0, i)}<u>{label[i]}</u>{label.slice(i + 1)}</>;
}

/** Hide the session (the card menu's HIDE), with Undo. */
export function hideSession(store: Store, id: string) {
  void store.setHidden(id, true).then(() => {
    store.toast('Session hidden', 'info', { label: 'UNDO', run: () => { void store.setHidden(id, false); } });
    if (store.selected.value === id) store.close();
  });
}

/** Move the open session; Done offers Undo, like the card menu. */
export function quickMove(store: Store, d: CardDetail, to: MoveTarget) {
  if (to === d.column) return;
  const back: MoveTarget = d.manual ? (d.column as MoveTarget) : 'auto';
  void store.move(d.id, to).then(() => {
    if (to === 'done') store.toast('Moved to Done', 'info', { label: 'UNDO', run: () => { void store.move(d.id, back); } });
  });
}

/** One click moves the open session; the current column is pressed. RESUME sits apart at the right end (R). */
export function QuickMove({ d, store }: { d: CardDetail; store: Store }) {
  const go = (to: MoveTarget) => quickMove(store, d, to);
  return (
    <div class="qmove" role="group" aria-label="Move to">
      {QUICK.map((m) => (
        <button key={m.to} class={`c-${m.to}`} aria-pressed={d.column === m.to} aria-keyshortcuts={m.key.toUpperCase()} title={`${m.label} (${m.key.toUpperCase()})`} onClick={() => go(m.to)}>
          <Keyed label={m.label} k={m.key} />
        </button>
      ))}
      {d.manual && <button class="auto" onClick={() => go('auto')} title="Let the rules place it again">AUTO</button>}
      {d.resumable && <button class="resume" aria-keyshortcuts="R" title="Resume (R)" onClick={() => void store.resume(d.id)}><u>R</u>ESUME</button>}
    </div>
  );
}

