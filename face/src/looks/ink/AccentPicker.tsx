import { useEffect, useRef, useState } from 'preact/hooks';
import { ACCENTS } from '../../accents';
import type { Store } from '../../store';

/** One header button that wears the current accent; it opens a row of the eight to choose from. */
export function AccentPicker({ store }: { store: Store }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const accent = store.accent.value;
  const current = ACCENTS.find((a) => a.id === accent);

  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => { if (!root.current?.contains(e.target as Node)) setOpen(false); };
    const esc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      setOpen(false);
      button.current?.focus();
    };
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', esc, true);
    return () => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', esc, true);
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
        title="Accent colour"
        aria-haspopup="true"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      ><i /></button>
      {open && (
        <div class="accpop" role="group" aria-label="Accent colour">
          {ACCENTS.map((a) => (
            <button key={a.id} type="button" class="acc" data-accent={a.id} aria-label={a.name} aria-pressed={accent === a.id} title={a.name} onClick={() => pick(a.id)} />
          ))}
        </div>
      )}
    </div>
  );
}
