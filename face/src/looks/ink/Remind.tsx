import { useEffect, useId, useRef } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import { Pix } from '../../pixel/Pix';
import type { Store } from '../../store';
import type { Card } from '../../api/types';
import { remindText } from '../../format';
import { ICONS, INK } from './art';
import { addDays, dateValue, DatePicker, parseTime, TimeField } from './DatePick';
import { Sec } from './Sections';

/** The card whose reminder form is open (a date, a time and a note), or null. */
export const remindForm = signal<string | null>(null);

/** The quick choices, in the words `eye remind` reads. */
export const REMIND_PRESETS: ReadonlyArray<{ label: string; when: string }> = [
  { label: 'TOMORROW 9:00', when: 'tomorrow' },
  { label: 'MONDAY 9:00', when: 'mon' },
  { label: 'IN A WEEK', when: '1w' },
];

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

/** Why the form cannot be sent yet, or null: a time that does not read, or one that has passed. */
export function whenProblem(date: string, time: string, now = Date.now()): string | null {
  const t = time.trim() ? parseTime(time) : '09:00';
  if (!t) return 'Write the time as HH:MM, 24 h, e.g. 14:30.';
  const [y, m, d] = date.split('-').map(Number);
  const [h, min] = t.split(':').map(Number);
  return new Date(y!, m! - 1, d!, h, min).getTime() <= now ? 'That time has passed. Pick a later one.' : null;
}

/** DESKTOP NOTICE ON | OFF, for every reminder; hidden when this machine cannot show one or the engine has no such setting. */
function NoticeSwitch({ store }: { store: Store }) {
  useEffect(() => { store.loadNotice(); }, []);
  const lbl = useId();
  const n = store.notice.value;
  if (!n?.available) return null;
  return (
    <div class="notice">
      <div class="nrow">
        <span id={lbl}>DESKTOP NOTICE</span>
        <div class="theme seg" role="group" aria-labelledby={lbl}>
          {([true, false] as const).map((on) => (
            <button key={String(on)} type="button" class={n.notify === on ? 'on' : ''} aria-pressed={n.notify === on} onClick={() => { if (n.notify !== on) void store.setNotice(on); }}>{on ? 'ON' : 'OFF'}</button>
          ))}
        </div>
      </div>
      <p class="nhint">Applies to every reminder on this computer.</p>
    </div>
  );
}

/** Pick a date, a time and a note for a reminder. */
function RemindForm({ store, id, onClose }: { store: Store; id: string; onClose: () => void }) {
  const today = dateValue(new Date());
  const date = useSignal(addDays(today, 1));
  const time = useSignal('09:00');
  const problem = useSignal<string | null>(null);
  const busy = useSignal(false);
  const form = useRef<HTMLFormElement>(null);
  useEffect(() => { form.current?.querySelector<HTMLButtonElement>('.dpgrid button[tabindex="0"]')?.focus(); }, []);
  const submit = async (e: Event) => {
    e.preventDefault();
    if (busy.value) return;
    problem.value = whenProblem(date.value, time.value);
    if (problem.value) { form.current?.querySelector<HTMLInputElement>('#remind-time')?.focus(); return; }
    const note = (form.current?.elements.namedItem('note') as HTMLInputElement).value.trim();
    busy.value = true;
    const ok = await store.remind(id, { when: whenText(date.value, parseTime(time.value) ?? ''), note: note || undefined });
    busy.value = false;
    if (ok) onClose();
  };
  return (
    <>
      <div class="scrim" onClick={onClose} />
      <form ref={form} class="modal" onSubmit={submit} aria-label="Remind me">
        <div class="dis" />
        <h2><Pix rows={ICONS.needs_you} pal={INK} scale={4} /> REMIND ME</h2>
        <div class="when">
          <div class="wdate">
            <span class="flbl" id="remind-date">DATE</span>
            <DatePicker value={date.value} min={today} labelId="remind-date" onChange={(d) => { date.value = d; problem.value = null; }} />
          </div>
          <div class="wtime">
            <label class="flbl" for="remind-time">TIME</label>
            <TimeField id="remind-time" value={time.value} invalid={!!problem.value} onChange={(t) => { time.value = t; problem.value = null; }} />
            {problem.value && <p class="fnote" id="remind-time-err" role="alert">{problem.value}</p>}
          </div>
        </div>
        <label>NOTE<span class="hint">Optional. Shown on the card when it is due.</span>
          <input name="note" placeholder="Ana's reply" autocomplete="off" />
        </label>
        <NoticeSwitch store={store} />
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
      <NoticeSwitch store={store} />
    </Sec>
  );
}
