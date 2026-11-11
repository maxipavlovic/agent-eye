import { useEffect, useLayoutEffect, useRef } from 'preact/hooks';
import { signal } from '@preact/signals';
import type { Store } from '../../store';
import type { Card, ColumnId, MoveTarget } from '../../api/types';
import { copy } from './copy';
import { startButton } from './folder';
import { ideaForm } from './IdeaForm';
import { openRemind, REMIND_PRESETS } from './Remind';

interface Open {
  seq: number;
  card: Card;
  x: number;
  y: number;
  /** Opened from the keyboard: Esc gives the focus back to the card. */
  from: HTMLElement | null;
}

const menu = signal<Open | null>(null);
let seq = 0;

function show(card: Card, x: number, y: number, from: HTMLElement | null = null) {
  menu.value = { seq: ++seq, card, x, y, from };
}

function close(restore = false) {
  const m = menu.value;
  menu.value = null;
  if (restore) m?.from?.focus();
}

/** A finger held this long opens the menu instead of the details. */
const HOLD_MS = 450;
let hold = 0;
let held = -1e9;
let at = { x: 0, y: 0 };

/** Everything a card needs to be a button that opens details and a right-click / long-press menu. */
export function cardProps(c: Card, store: Store) {
  const openIt = () => store.open(c.id);
  return {
    tabIndex: 0,
    role: 'button',
    onClick: () => { if (performance.now() - held > 800) openIt(); },
    onKeyDown: (e: KeyboardEvent) => {
      if (e.target !== e.currentTarget) return;
      const el = e.currentTarget as HTMLElement;
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openIt(); }
      else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
        e.preventDefault();
        const r = el.getBoundingClientRect();
        show(c, r.left + 24, r.top + Math.min(r.height, 40), el);
      }
    },
    onContextMenu: (e: MouseEvent) => { e.preventDefault(); show(c, e.clientX, e.clientY); },
    onPointerDown: (e: PointerEvent) => {
      if (e.pointerType !== 'touch') return;
      at = { x: e.clientX, y: e.clientY };
      clearTimeout(hold);
      hold = window.setTimeout(() => { held = performance.now(); show(c, at.x, at.y); }, HOLD_MS);
    },
    onPointerMove: (e: PointerEvent) => { if (Math.hypot(e.clientX - at.x, e.clientY - at.y) > 10) clearTimeout(hold); },
    onPointerUp: () => clearTimeout(hold),
    onPointerCancel: () => clearTimeout(hold),
  } as Record<string, unknown>;
}

export type Row = { hr: true } | { head: string } | { label: string; run: () => void; sub?: boolean };

const MOVE_TO: Array<{ to: ColumnId; label: string }> = [
  { to: 'needs_you', label: 'NEEDS YOU' },
  { to: 'waiting', label: 'WAITING' },
  { to: 'paused', label: 'PAUSED' },
];

/** Resolves once the action settled, then says whether the board shows what the action wanted. */
function onBoard(store: Store, id: string): ColumnId | null {
  for (const col of store.board.value?.columns ?? []) if (col.cards.some((x) => x.id === id)) return col.id;
  return null;
}

/** REMIND ME…: the quick choices, a date to pick, and clearing one that is set. */
export function remindRows(c: Card, store: Store, go: (fn: () => void) => () => void): Row[] {
  const out: Row[] = [{ head: 'REMIND ME…' }];
  for (const p of REMIND_PRESETS) out.push({ label: p.label, sub: true, run: go(() => { void store.remind(c.id, { when: p.when }); }) });
  out.push({ label: 'PICK A DATE…', sub: true, run: go(() => { openRemind(c.id); }) });
  if (c.remind_at) out.push({ label: 'CLEAR REMINDER', sub: true, run: go(() => { void store.remind(c.id, { clear: true }); }) });
  return out;
}

function rows(c: Card, store: Store): Row[] {
  const go = (fn: () => void) => () => { close(); fn(); };
  const open: Row = { label: 'OPEN DETAILS', run: go(() => store.open(c.id)) };
  if (c.kind === 'idea') {
    const edit: Row = { label: 'EDIT', run: go(() => { store.open(c.id); ideaForm.value = { edit: c.id }; }) };
    const start = startButton(c);
    return [...(start.blocked ? [] : [{ label: start.label, run: go(() => { void store.startIdea(c, false); }) }]), edit, { hr: true }, ...remindRows(c, store, go), { hr: true }, open];
  }
  const out: Row[] = [];
  if (c.column !== 'done') {
    // Undo puts it back where it was: a hand-placed column, or back to the rules.
    const back: MoveTarget = c.manual ? (c.column as MoveTarget) : 'auto';
    out.push({
      label: 'MARK DONE',
      run: go(() => {
        void store.move(c.id, 'done').then(() => {
          if (onBoard(store, c.id) !== 'done') return;
          store.toast('Moved to Done', 'info', { label: 'UNDO', run: () => { void store.move(c.id, back); } });
        });
      }),
    });
  }
  const moves = MOVE_TO.filter((m) => m.to !== c.column);
  if (moves.length) {
    out.push({ head: 'MOVE TO' });
    for (const m of moves) out.push({ label: m.label, sub: true, run: go(() => { void store.move(c.id, m.to as MoveTarget); }) });
  }
  out.push({ hr: true });
  out.push(...remindRows(c, store, go));
  out.push({ hr: true });
  out.push({
    label: 'HIDE (USELESS)',
    run: go(() => {
      void store.setHidden(c.id, true).then(() => {
        if (onBoard(store, c.id) !== null) return;
        store.toast('Session hidden', 'info', { label: 'UNDO', run: () => { void store.setHidden(c.id, false); } });
      });
    }),
  });
  if (c.resume_cmd) out.push({ label: 'COPY RESUME COMMAND', run: go(() => copy(c.resume_cmd!, store)) });
  if (c.resume_cmd) out.push({ label: 'FORK', run: go(() => { void store.resume(c.id, true); }) });
  out.push(open);
  return out;
}

export function CardMenu({ store }: { store: Store }) {
  const m = menu.value;
  const ref = useRef<HTMLDivElement>(null);

  // Place it at the pointer, flipped and clamped so it stays inside the viewport; focus the first item.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!m || !el) return;
    const r = el.getBoundingClientRect();
    const pad = 8;
    let left = m.x + r.width + pad > innerWidth ? m.x - r.width : m.x;
    let top = m.y + r.height + pad > innerHeight ? m.y - r.height : m.y;
    left = Math.max(pad, Math.min(left, innerWidth - r.width - pad));
    top = Math.max(pad, Math.min(top, innerHeight - r.height - pad));
    el.style.left = `${left}px`;
    el.style.top = `${top}px`;
    el.style.visibility = 'visible';
    el.querySelector<HTMLElement>('button')?.focus({ preventScroll: true });
  }, [m?.seq]);

  useEffect(() => {
    if (!m) return;
    const items = () => [...(ref.current?.querySelectorAll<HTMLElement>('button') ?? [])];
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(true); return; }
      if (e.key === 'Tab') { close(); return; }
      const step = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : 0;
      if (!step && e.key !== 'Home' && e.key !== 'End') return;
      e.preventDefault();
      e.stopPropagation();
      const list = items();
      const i = list.indexOf(document.activeElement as HTMLElement);
      const next = e.key === 'Home' ? 0 : e.key === 'End' ? list.length - 1 : i < 0 ? (step > 0 ? 0 : list.length - 1) : (i + step + list.length) % list.length;
      list[next]?.focus({ preventScroll: true });
    };
    const onScroll = (e: Event) => { if (!ref.current?.contains(e.target as Node)) close(); };
    const shut = () => close();
    addEventListener('keydown', onKey, true);
    addEventListener('scroll', onScroll, true);
    addEventListener('resize', shut);
    addEventListener('blur', shut);
    return () => {
      removeEventListener('keydown', onKey, true);
      removeEventListener('scroll', onScroll, true);
      removeEventListener('resize', shut);
      removeEventListener('blur', shut);
    };
  }, [m?.seq]);

  if (!m) return null;
  return (
    <>
      <div
        class="ctxscrim"
        onPointerDown={() => close()}
        onContextMenu={(e) => {
          e.preventDefault();
          close();
          // Right-clicking another card while a menu is open opens that card's menu.
          const { clientX, clientY } = e;
          requestAnimationFrame(() => {
            document.elementFromPoint(clientX, clientY)?.dispatchEvent(
              new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX, clientY }),
            );
          });
        }}
      />
      <div class="ctx" key={m.seq} ref={ref} role="menu" aria-label={`Actions for ${m.card.title}`} style={`left:${m.x}px;top:${m.y}px;visibility:hidden`}>
        <p class="ct">{m.card.title}</p>
        {rows(m.card, store).map((r, i) =>
          'hr' in r ? <hr key={i} />
            : 'head' in r ? <p class="gl" key={i} role="presentation">{r.head}</p>
              : (
                <button key={i} role="menuitem" class={r.sub ? 'sub' : ''} onClick={r.run} onPointerMove={(e) => { if (e.pointerType === 'mouse') e.currentTarget.focus({ preventScroll: true }); }}>{r.label}</button>
              ))}
      </div>
    </>
  );
}
