import { ACCENTS, DEFAULT_ACCENT, parseAccent, type AccentId } from '../../face/src/accents';

// The board uses the same key, so the demo opens in the accent chosen here.
const ACCENT_KEY = 'eye.accent';

function savedAccent(): AccentId {
  try { return parseAccent(localStorage.getItem(ACCENT_KEY)); } catch { /* storage blocked */ }
  return DEFAULT_ACCENT;
}

function applyAccent(id: AccentId) {
  const set = (root: HTMLElement) => {
    if (id === DEFAULT_ACCENT) delete root.dataset.accent;
    else root.dataset.accent = id;
  };
  set(document.documentElement);
  // The demo board is on the same origin: keep it in step.
  try { const d = document.querySelector('iframe')?.contentDocument?.documentElement; if (d) set(d); } catch { /* not loaded yet */ }
}

/** The header's accent button: it wears the current accent and opens a row of the eight, as on the board. */
export function setupAccent() {
  const root = document.querySelector<HTMLElement>('.accpick');
  const button = root?.querySelector<HTMLButtonElement>('.accbtn');
  if (!root || !button) return;
  let accent = savedAccent();
  let pop: HTMLElement | null = null;

  const label = () => `Accent colour: ${ACCENTS.find((a) => a.id === accent)?.name ?? accent}`;
  const sync = () => {
    applyAccent(accent);
    button.setAttribute('aria-label', label());
    for (const s of root.querySelectorAll<HTMLElement>('.acc')) s.setAttribute('aria-pressed', String(s.dataset.accent === accent));
  };
  const close = (refocus = false) => {
    pop?.remove();
    pop = null;
    button.setAttribute('aria-expanded', 'false');
    if (refocus) button.focus();
  };
  const away = (e: PointerEvent) => { if (!root.contains(e.target as Node)) close(); };
  const esc = (e: KeyboardEvent) => { if (e.key === 'Escape' && pop) { e.stopPropagation(); close(true); } };
  // tabbing out of the picker closes it too
  root.addEventListener('focusout', (e) => { if (pop && !root.contains(e.relatedTarget as Node | null)) close(); });
  document.addEventListener('pointerdown', away);
  document.addEventListener('keydown', esc, true);

  const open = () => {
    pop = document.createElement('div');
    pop.className = 'accpop';
    pop.setAttribute('role', 'group');
    pop.setAttribute('aria-label', 'Accent colour');
    for (const a of ACCENTS) {
      const s = document.createElement('button');
      s.type = 'button';
      s.className = 'acc';
      s.dataset.accent = a.id;
      s.setAttribute('aria-label', a.name);
      s.title = a.name;
      s.addEventListener('click', () => {
        accent = a.id;
        try { localStorage.setItem(ACCENT_KEY, accent); } catch { /* storage blocked */ }
        sync();
        close(true);
      });
      pop.append(s);
    }
    root.append(pop);
    button.setAttribute('aria-expanded', 'true');
    sync();
  };
  button.addEventListener('click', () => (pop ? close() : open()));
  document.querySelector('iframe')?.addEventListener('load', sync);
  sync();
}
