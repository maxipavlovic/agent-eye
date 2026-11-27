import { useEffect, useRef } from 'preact/hooks';
import { useSignal } from '@preact/signals';
import type { ModelSetting, SettingsInput } from '../api/types';
import { restartNote } from './SettingsParts';

/** The value of "Other…" in the choice: never a model name, which has no spaces. */
const OTHER = ' other';

/**
 * The choice's options before "Other…", as [value, text]: the agent's default (an empty value), each other name it
 * takes, and the value now when it is none of those. An engine that sends no names gets the default alone.
 */
export function modelOptions(m: ModelSetting): Array<[string, string]> {
  const def = m.default ?? '';
  const named = (m.choices ?? []).filter((c) => c !== def);
  const out: Array<[string, string]> = [['', def ? `Default: ${def}` : 'Default'], ...named.map((c): [string, string] => [c, c])];
  if (m.value && !named.includes(m.value)) out.push([m.value, m.value]);
  return out;
}

/**
 * The model summaries are written with: a choice of the names the summary agent takes, saved at once, and "Other…",
 * which shows a field for any other name.
 */
export function ModelPick({ m, sample, save }: { m: ModelSetting; sample: boolean; save: (input: SettingsInput) => Promise<boolean> }) {
  const other = useSignal(false);
  const draft = useSignal('');
  const field = useRef<HTMLInputElement>(null);
  useEffect(() => { if (other.value) field.current?.focus(); }, [other.value]);
  const options = modelOptions(m);
  const pick = async (e: Event) => {
    const select = e.currentTarget as HTMLSelectElement;
    const v = select.value;
    if (v === OTHER) {
      // a name of its own is there to edit; a listed one is in the choice already
      draft.value = (m.choices ?? []).includes(m.value) ? '' : m.value;
      other.value = true;
      return;
    }
    other.value = false;
    if (v !== m.value && !(await save({ 'summary.model': v }))) select.value = m.value;
  };
  const submit = async (e: Event) => {
    e.preventDefault();
    const v = draft.value.trim();
    if (v === m.value || (await save({ 'summary.model': v }))) other.value = false;
  };
  return (
    <form class="nset" onSubmit={submit}>
      <label>MODEL FOR SUMMARIES
        <select value={other.value ? OTHER : m.value} disabled={sample} onChange={pick}>
          {options.map(([v, text]) => <option key={v} value={v}>{text}</option>)}
          <option value={OTHER}>Other…</option>
        </select>
      </label>
      {other.value && (
        <>
          <label>MODEL NAME
            <input ref={field} value={draft.value} maxLength={m.max} disabled={sample} onInput={(e) => { draft.value = e.currentTarget.value; }} placeholder="empty: the default" autocomplete="off" spellcheck={false} />
          </label>
          <button class="btn sm" type="submit" disabled={sample}>SET</button>
        </>
      )}
      <span class="hint">The model {m.agent ?? 'your agent'} writes summaries (and judges corrections) with. Small is fine: a summary is short.{restartNote(m.restart)}</span>
    </form>
  );
}
