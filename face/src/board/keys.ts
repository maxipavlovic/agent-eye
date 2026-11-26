import { useEffect, useRef } from 'preact/hooks';
import type { Store } from '../store';
import { copy } from './copy';
import { exportSession, ideaAct } from './Detail';
import { ideaForm } from './IdeaForm';
import { closeForget, forgetFor, openForget } from './Forget';
import { keyHelp } from './KeyHelp';
import { leftoversOpen } from './Loose';
import { hideSession, QUICK, quickMove } from './Move';
import { canContinueTonight, canPlanIdea, nightAsk, tonightOpen } from './Night';
import { closeRemind, openRemind, remindForm } from './Remind';
import { settingsPanel } from './Settings';
import type { Walk } from './walk';

/** Board keys: / or ⌘K / Ctrl-K searches, ← → walk the cards, Esc backs out one level, ? lists the keys. The open session's panel
 *  takes R D W P N M T F C H L E X (see KEY_LIST), an idea's S B T E M (IDEA_KEY_LIST), and only while no field has the focus. */
/** Where `key` scrolls `list` to: a line, most of a page or an end; null for any other key. */
function scrollTarget(list: HTMLElement, key: string): number | null {
  const at = list.scrollTop, page = list.clientHeight * 0.9;
  const to: Record<string, number> = { ArrowDown: at + 40, ArrowUp: at - 40, PageDown: at + page, PageUp: at - page, Home: 0, End: list.scrollHeight };
  return Object.hasOwn(to, key) ? to[key]! : null;
}

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
      // the keys list takes Esc and ? while it is open, and the arrow, page, Home and End keys scroll it
      // (nothing in it takes the focus, so Safari would not scroll it)
      if (keyHelp.value) {
        if (e.key === 'Escape' || e.key === '?') { e.preventDefault(); e.stopImmediatePropagation(); keyHelp.value = false; }
        const list = document.querySelector<HTMLElement>('.keyhelp .mbody');
        const top = list && scrollTarget(list, e.key);
        if (top != null) { e.preventDefault(); list!.scrollTo({ top }); }
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
      if (e.key === ',' && !typing && !ideaForm.value && plain) { e.preventDefault(); settingsPanel.value = !settingsPanel.value; return; }
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

/** The panel keys that are not moves: remind, continue tonight, fork, copy, hide, leftovers, export, forget; an idea's
 *  start, in the background, tonight, edit and remind. True when the key was one of them and did something (or
 *  deliberately nothing); the buttons carry the same keys. */
function panelAct(store: Store, d: NonNullable<Store['detail']['value']>, k: string): boolean {
  if (d.kind === 'idea') return ideaKey(store, d, k);
  switch (k) {
    case 'm': openRemind(d.id); return true;
    case 't':
      if (!canContinueTonight(d, store)) return false;
      tonightOpen.value = d.id;
      return true;
    case 'f':
      if (!d.resumable || !d.resume_cmd) return false;
      void store.resume(d.id, true);
      return true;
    case 'c':
      if (!d.resume_cmd) return false;
      copy(d.resume_cmd, store);
      return true;
    case 'h':
      hideSession(store, d.id);
      return true;
    case 'l': {
      if (!d.loose_ends?.length) return false;
      leftoversOpen.value = d.id;
      requestAnimationFrame(() => document.getElementById('leftovers')?.scrollIntoView({ block: 'start', behavior: 'smooth' }));
      return true;
    }
    case 'e':
      exportSession(d.id);
      return true;
    case 'x':
      openForget(d);
      return true;
    default: return false;
  }
}

/** An idea's panel keys: S start, B in the background, T tonight, E edit, M remind me. */
function ideaKey(store: Store, d: NonNullable<Store['detail']['value']>, k: string): boolean {
  switch (k) {
    case 's': return ideaAct(store, d, 'start');
    case 'b': return ideaAct(store, d, 'background');
    case 'e': return ideaAct(store, d, 'edit');
    case 't':
      if (!canPlanIdea(d, store)) return false;
      nightAsk.value = d.id;
      return true;
    case 'm': openRemind(d.id); return true;
    default: return false;
  }
}
