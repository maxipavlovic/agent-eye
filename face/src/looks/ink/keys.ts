import { useEffect, useRef } from 'preact/hooks';
import type { Store } from '../../store';
import { ideaForm } from './IdeaForm';
import { QUICK, quickMove } from './Move';
import { closeRemind, remindForm } from './Remind';
import type { Walk } from './walk';

/** Board keys: / or ⌘K / Ctrl-K searches, N W P D move the open session, R resumes it, ← → walk the cards, Esc backs out one level. */
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
      // the reminder form takes the keys while it is open; Esc closes it
      if (remindForm.value) {
        if (e.key === 'Escape') closeRemind();
        return;
      }
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
