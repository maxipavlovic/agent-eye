import { useLayoutEffect, useRef, useState } from 'preact/hooks';
import { ACCENTS } from '../accents';
import { LOOK_LIST } from '../looks';
import { savePref } from '../prefs';
import type { Store } from '../store';

/**
 * The page's look switches by loading the page again (each look brings its own styles), without a `?look=`
 * that would pick the old one. When the engine refuses the setting, the page stays and a toast says so.
 */
async function switchLook(store: Store, id: string) {
  if (!(await savePref('eye.look', id))) {
    store.toast('The look could not be saved: eye refused it. Try again, or run eye doctor.', 'error');
    return;
  }
  const url = new URL(location.href);
  if (url.searchParams.has('look')) {
    url.searchParams.delete('look');
    location.replace(url.toString());
  } else {
    location.reload();
  }
}

/**
 * One header button that wears the current accent; it opens the looks and a row of the eight accents to choose from.
 * The offline sample board has one look and no `data-look`, so it shows only the accents.
 */
export function AccentPicker({ store }: { store: Store }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const accent = store.accent.value;
  const current = ACCENTS.find((a) => a.id === accent);
  const look = document.documentElement.dataset.look;

  // listening from the moment the row is drawn, so an Esc or a click right after opening is not lost
  useLayoutEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => { if (!root.current?.contains(e.target as Node)) setOpen(false); };
    const esc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      setOpen(false);
      button.current?.focus();
    };
    // tabbing out of the picker closes it too
    const left = (e: FocusEvent) => { if (!root.current?.contains(e.relatedTarget as Node | null)) setOpen(false); };
    const el = root.current;
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', esc, true);
    el?.addEventListener('focusout', left);
    return () => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', esc, true);
      el?.removeEventListener('focusout', left);
    };
  }, [open]);

  const pick = (id: (typeof ACCENTS)[number]['id']) => {
    store.setAccent(id);
    setOpen(false);
    button.current?.focus();
  };

  return (
    <div class="accpick" ref={root}>
      <button
        ref={button}
        type="button"
        class="accbtn"
        data-accent={accent}
        aria-label={`Accent colour: ${current?.name ?? accent}`}
        title={look ? 'Look and accent colour' : 'Accent colour'}
        aria-haspopup="true"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      ><i /></button>
      {open && (
        <div class="accpop">
          {look && (
            <div class="looks" role="group" aria-label="Look">
              {LOOK_LIST.map((l) => (
                <button key={l.id} type="button" class="lookbtn" aria-pressed={look === l.id} title={l.title} onClick={() => { if (look !== l.id) void switchLook(store, l.id); }}>{l.label}</button>
              ))}
            </div>
          )}
          <div class="accs" role="group" aria-label="Accent colour">
            {ACCENTS.map((a) => (
              <button key={a.id} type="button" class="acc" data-accent={a.id} aria-label={a.name} aria-pressed={accent === a.id} title={a.name} onClick={() => pick(a.id)} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
