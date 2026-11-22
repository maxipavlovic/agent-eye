import { useEffect, useRef } from 'preact/hooks';
import type { Store } from '../../store';
import { copy } from './copy';
import { exportSession } from './Detail';
import { ideaForm } from './IdeaForm';
import { closeForget, forgetFor, openForget } from './Forget';
import { keyHelp } from './KeyHelp';
import { leftoversOpen } from './Loose';
import { hideSession, QUICK, quickMove } from './Move';
import { canContinueTonight, tonightOpen } from './Night';
import { closeRemind, openRemind, remindForm } from './Remind';
import type { Walk } from './walk';

/** Board keys: / or ⌘K / Ctrl-K searches, ← → walk the cards, Esc backs out one level, ? lists the keys. The open session's panel
 *  takes R D W P N M T F C H L E X (see KEY_LIST), and only while no field has the focus. */
/** Back to the search box, with what is in it selected, so typing starts a new search. */
export function focusSearch() {
  const q = document.getElementById('q') as HTMLInputElement | null;
  q?.focus();
  q?.select();
}

export function useKeys(store: Store, step: Walk['step']) {
  const stepRef = useRef(step);
  stepRef.current = step;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      const typing = t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT');
      const plain = !e.metaKey && !e.ctrlKey && !e.altKey;
      const d = store.detail.value;
      const quick = QUICK.find((m) => m.key === e.key.toLowerCase());
      // the keys list takes Esc and ? while it is open
      if (keyHelp.value) {
        if (e.key === 'Escape' || e.key === '?') { e.preventDefault(); e.stopImmediatePropagation(); keyHelp.value = false; }
        return;
      }
      // the reminder form takes the keys while it is open; Esc closes it
      if (remindForm.value) {
        if (e.key === 'Escape') closeRemind();
        return;
      }
      // the FORGET… confirm takes the keys while it is open; Esc closes it
      if (forgetFor.value) {
        if (e.key === 'Escape') { e.preventDefault(); closeForget(); }
        return;
      }
      const panelKey = !typing && !ideaForm.value && !!store.selected.value && !!d && plain && !e.shiftKey ? e.key.toLowerCase() : '';
      if (e.key === '?' && !typing && !ideaForm.value && plain) { e.preventDefault(); keyHelp.value = true; return; }
      if (panelKey && d && panelAct(store, d, panelKey)) { e.preventDefault(); return; }
      const findKey = (e.key === '/' && !typing && !e.metaKey && !e.ctrlKey)
        || (e.key.toLowerCase() === 'k' && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && !ideaForm.value);
      if (findKey) { e.preventDefault(); focusSearch(); }
      else if (!typing && !ideaForm.value && store.selected.value && d && d.kind !== 'idea' && plain && quick) {
        e.preventDefault();
        quickMove(store, d, quick.to);
      }
      else if (!typing && !ideaForm.value && store.selected.value && d?.resumable && plain && !e.shiftKey && e.key.toLowerCase() === 'r') {
        e.preventDefault();
        void store.resume(d.id);
      }
      else if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && !typing && !ideaForm.value && store.selected.value && plain && !e.shiftKey) {
        e.preventDefault();
        stepRef.current(e.key === 'ArrowRight' ? 1 : -1);
      }
      else if (e.key === 'Escape') {
        if (ideaForm.value) ideaForm.value = null;
        else if (store.selected.value) store.close();
        else if (store.query.value) { store.search(''); }
        else (document.activeElement as HTMLElement | null)?.blur();
      }
    };
    addEventListener('keydown', onKey);
    return () => removeEventListener('keydown', onKey);
  }, []);
}

/** The panel keys that are not moves: remind, continue tonight, fork, copy, hide, leftovers, export, forget.
 *  True when the key was one of them and did something (or deliberately nothing); the buttons carry the same keys. */
function panelAct(store: Store, d: NonNullable<Store['detail']['value']>, k: string): boolean {
  const session = d.kind !== 'idea';
  switch (k) {
    case 'm': openRemind(d.id); return true;
    case 't':
      if (!canContinueTonight(d, store)) return false;
      tonightOpen.value = d.id;
      return true;
    case 'f':
      if (!session || !d.resumable || !d.resume_cmd) return false;
      void store.resume(d.id, true);
      return true;
    case 'c':
      if (!d.resume_cmd) return false;
      copy(d.resume_cmd, store);
      return true;
    case 'h':
      if (!session) return false;
      hideSession(store, d.id);
      return true;
    case 'l': {
      if (!session || !d.loose_ends?.length) return false;
      leftoversOpen.value = d.id;
      requestAnimationFrame(() => document.getElementById('leftovers')?.scrollIntoView({ block: 'start', behavior: 'smooth' }));
      return true;
    }
    case 'e':
      if (!session) return false;
      exportSession(d.id);
      return true;
    case 'x':
      if (d.kind === 'idea') return false;
      openForget(d);
      return true;
    default: return false;
  }
}
