import type { Store } from '../store';
import { needsLane } from './gaze';
import { hoursPanel } from './Hours';
import { ideaForm } from './IdeaForm';
import { TonightChip } from './Night';
import { settingsPanel } from './Settings';
import { usagePanel } from './Usage';

// The header's controls, the same in every look; a look's Header arranges them and draws their icons.

const CONN: Record<string, string> = { live: 'LIVE', connecting: 'LINKING', offline: 'OFFLINE', fixture: 'SAMPLE' };

/** The search field, with its `/` key before the field or after it. */
export function SearchField({ store, keyAfter }: { store: Store; keyAfter?: boolean }) {
  const key = <kbd>/</kbd>;
  return (
    <label class="search" title="Search (/ or ⌘K)">
      {!keyAfter && key}
      <input id="q" type="search" placeholder="Search every session" aria-label="Search every session" autocomplete="off" aria-keyshortcuts="/ Meta+K Control+K" value={store.query.value} onInput={(e) => store.search(e.currentTarget.value)} />
      {keyAfter && key}
    </label>
  );
}

/** NEEDS YOU and how many (ALL CLEAR when none), which brings the Needs you lane into view. */
export function NeedsButton({ store }: { store: Store }) {
  const need = store.needsYou.value;
  return (
    <button class={`needbtn${need ? ' on' : ''}`} onClick={() => needsLane()?.scrollIntoView({ inline: 'center', block: 'nearest' })}>
      <span>{need ? 'NEEDS YOU' : 'ALL CLEAR'}</span><b>{need}</b>
    </button>
  );
}

/** Where the board's data comes from: live, linking, offline or the sample. */
export function ConnState({ store }: { store: Store }) {
  const conn = store.connection.value;
  return <span class={`conn ${conn}`} title={`data: ${conn}`}><i />{CONN[conn] ?? conn}</span>;
}

/** USAGE: what the sessions used and cost, and where the plan limits stand. */
export function UsageButton() {
  return <button class="btn" title="What the sessions used and cost, and where the plan limits stand" aria-haspopup="dialog" onClick={() => { usagePanel.value = !usagePanel.value; }}>USAGE</button>;
}

/** HOURS: when you steer well or badly, and whether it is you or the model. */
export function HoursButton() {
  return <button class="btn" title="When you steer well or badly, and whether it is you or the model" aria-haspopup="dialog" onClick={() => { hoursPanel.value = !hoursPanel.value; }}>HOURS</button>;
}

/** The quiet mark beside HOURS while the clock has a hint (`[clock] hints`): its words are the label and the tooltip, and it opens HOURS. */
export function HoursHint({ store }: { store: Store }) {
  const hint = store.clockHint.value?.text;
  if (!hint) return null;
  return <button class="hourhint" title={hint} onClick={() => { hoursPanel.value = true; }}><i aria-hidden="true" /><span>{hint}</span></button>;
}

/** The gear: it opens SETTINGS (look, colour, layout, theme and the agent settings). */
export function SettingsGear() {
  return (
    <button class="btn gear" title="Settings (,)" aria-label="Settings" aria-haspopup="dialog" aria-keyshortcuts="," aria-pressed={settingsPanel.value} onClick={() => { settingsPanel.value = !settingsPanel.value; }}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" aria-hidden="true">
        <path d="M10.17 4.62 L10.28 1.64 L13.72 1.64 L13.83 4.62 L15.92 5.49 L18.10 3.46 L20.54 5.90 L18.51 8.08 L19.38 10.17 L22.36 10.28 L22.36 13.72 L19.38 13.83 L18.51 15.92 L20.54 18.10 L18.10 20.54 L15.92 18.51 L13.83 19.38 L13.72 22.36 L10.28 22.36 L10.17 19.38 L8.08 18.51 L5.90 20.54 L3.46 18.10 L5.49 15.92 L4.62 13.83 L1.64 13.72 L1.64 10.28 L4.62 10.17 L5.49 8.08 L3.46 5.90 L5.90 3.46 L8.08 5.49Z" />
        <circle cx="12" cy="12" r="3" />
      </svg>
    </button>
  );
}

/** The end of the row: the night, USAGE, HOURS (with its hint), the gear and + IDEA. */
export function HeaderEnd({ store }: { store: Store }) {
  return (
    <>
      <TonightChip store={store} />
      <UsageButton />
      <HoursButton />
      <HoursHint store={store} />
      <SettingsGear />
      <button class="btn plus" onClick={() => { ideaForm.value = {}; }}>+ IDEA</button>
    </>
  );
}
