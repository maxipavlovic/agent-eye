import type { Store } from '../store';
import { needsLane } from './gaze';
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

/** The gear: it opens SETTINGS (look, colour, layout, theme and the agent settings). */
export function SettingsGear() {
  return (
    <button class="btn gear" title="Settings (,)" aria-label="Settings" aria-haspopup="dialog" aria-keyshortcuts="," aria-pressed={settingsPanel.value} onClick={() => { settingsPanel.value = !settingsPanel.value; }}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" aria-hidden="true">
        <circle cx="12" cy="12" r="3.5" />
        <path d="M12 2.5v3M12 18.5v3M2.5 12h3M18.5 12h3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M18.7 5.3l-2.1 2.1M7.4 16.6l-2.1 2.1" />
      </svg>
    </button>
  );
}

/** The end of the row: the night, USAGE, the gear and + IDEA. The slot right after USAGE is for HOURS. */
export function HeaderEnd({ store }: { store: Store }) {
  return (
    <>
      <TonightChip store={store} />
      <UsageButton />
      <SettingsGear />
      <button class="btn plus" onClick={() => { ideaForm.value = {}; }}>+ IDEA</button>
    </>
  );
}
