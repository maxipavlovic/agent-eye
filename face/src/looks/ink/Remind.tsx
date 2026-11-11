import { useEffect, useRef } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import { Pix } from '../../pixel/Pix';
import type { Store } from '../../store';
import type { Card } from '../../api/types';
import { remindText } from '../../format';
import { ICONS, INK } from './art';
import { Sec } from './Sections';

/** The card whose reminder form is open (a date, a time and a note), or null. */
export const remindForm = signal<string | null>(null);

/** The quick choices, in the words `eye remind` reads. */
export const REMIND_PRESETS: ReadonlyArray<{ label: string; when: string }> = [
  { label: 'TOMORROW 9:00', when: 'tomorrow' },
  { label: 'MONDAY 9:00', when: 'mon' },
  { label: 'IN A WEEK', when: '1w' },
];

const pad = (n: number) => String(n).padStart(2, '0');

/** `2026-11-20` for a date input. */
export function dateValue(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** What the form sends: the date and the time as the engine reads them; the time is 9:00 when empty. */
export function whenText(date: string, time: string): string {
  return `${date} ${time || '09:00'}`;
}

/** What the panel says about a card's reminder: "Tomorrow 9:00 · Ana's reply", "Due today 9:00 · …" once due. */
export function remindSummary(c: Card, now = Date.now()): string | null {
  const text = remindText(c, now);
  if (!text) return null;
  return c.remind_due ? `Due ${text}` : text[0]!.toUpperCase() + text.slice(1);
}

/** Pick a date, a time and a note for a reminder. */
function RemindForm({ store, id, onClose }: { store: Store; id: string; onClose: () => void }) {
  const date = useRef<HTMLInputElement>(null);
  const busy = useSignal(false);
  useEffect(() => { date.current?.focus(); }, []);
  const submit = async (e: Event) => {
    e.preventDefault();
    if (busy.value) return;
    const f = e.currentTarget as HTMLFormElement;
    const v = (n: string) => (f.elements.namedItem(n) as HTMLInputElement).value.trim();
    if (!v('date')) { date.current?.focus(); return; }
    busy.value = true;
    const ok = await store.remind(id, { when: whenText(v('date'), v('time')), note: v('note') || undefined });
    busy.value = false;
    if (ok) onClose();
  };
  return (
    <>
      <div class="scrim" onClick={onClose} />
      <form class="modal" onSubmit={submit} aria-label="Remind me">
        <div class="dis" />
        <h2><Pix rows={ICONS.needs_you} pal={INK} scale={4} /> REMIND ME</h2>
        <label>DATE<input ref={date} name="date" type="date" min={dateValue(new Date())} defaultValue={dateValue(new Date(Date.now() + 86400000))} /></label>
        <label>TIME<input name="time" type="time" defaultValue="09:00" /></label>
        <label>NOTE<span class="hint">Optional. Shown on the card when it is due.</span>
          <input name="note" placeholder="Ana's reply" autocomplete="off" />
        </label>
        <div class="acts end">
          <button type="button" class="btn ghost" onClick={onClose}>CANCEL</button>
          <button type="submit" class="btn big" disabled={busy.value}>SET REMINDER</button>
        </div>
      </form>
    </>
  );
}

/** The reminder form when asked for. */
export function RemindFormHost({ store }: { store: Store }) {
  const id = remindForm.value;
  if (!id) return null;
  return <RemindForm key={id} store={store} id={id} onClose={() => { remindForm.value = null; }} />;
}

/** The reminder of a card in its panel: what is set, the quick choices and a way to clear it. */
export function RemindPanel({ c, store }: { c: Card; store: Store }) {
  const summary = remindSummary(c);
  return (
    <Sec t="REMINDER">
      <p class="notes">{summary ?? 'No reminder.'}</p>
      {!summary && <p class="dim">eye moves this card to Needs you at the time you pick.</p>}
      <div class="acts">
        {REMIND_PRESETS.map((p) => <button key={p.when} class="btn sm" onClick={() => void store.remind(c.id, { when: p.when })}>{p.label}</button>)}
        <button class="btn sm" onClick={() => { remindForm.value = c.id; }}>PICK A DATE…</button>
        {c.remind_at && <button class="btn sm ghost" onClick={() => void store.remind(c.id, { clear: true })}>CLEAR</button>}
      </div>
    </Sec>
  );
}
