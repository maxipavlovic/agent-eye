import type { ComponentChildren } from 'preact';
import type { Store, ThemePref } from '../store';
import { AccentPicker } from './AccentPicker';
import { needsLane } from './gaze';
import { ideaForm } from './IdeaForm';
import type { Layout } from './layout';
import { TonightChip } from './Night';
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

/** Lanes or Grid, each drawn by the look. */
export function LayoutSwitch({ layout, setLayout, icon }: { layout: Layout; setLayout: (l: Layout) => void; icon: (l: Layout) => ComponentChildren }) {
  return (
    <div class="theme layout" role="group" aria-label="Layout">
      {(['lanes', 'grid'] as const).map((k) => (
        <button key={k} class={layout === k ? 'on' : ''} aria-pressed={layout === k} title={k === 'lanes' ? 'Lanes' : 'Grid'} onClick={() => setLayout(k)}>{icon(k)}</button>
      ))}
    </div>
  );
}

/** Light, dark or as the system is, each drawn by the look. */
export function ThemeSwitch({ store, icon }: { store: Store; icon: (t: ThemePref) => ComponentChildren }) {
  const pref = store.themePref.value;
  return (
    <div class="theme" role="group" aria-label="Theme">
      {(['light', 'dark', 'auto'] as const).map((k) => (
        <button key={k} class={pref === k ? 'on' : ''} aria-pressed={pref === k} title={k} onClick={() => store.setTheme(k)}>{icon(k)}</button>
      ))}
    </div>
  );
}

/** The end of the row: the look and colour, the night, USAGE and + IDEA. */
export function HeaderEnd({ store }: { store: Store }) {
  return (
    <>
      <AccentPicker store={store} />
      <TonightChip store={store} />
      <button class="btn" title="What the sessions used and cost, and where the plan limits stand" aria-haspopup="dialog" onClick={() => { usagePanel.value = !usagePanel.value; }}>USAGE</button>
      <button class="btn plus" onClick={() => { ideaForm.value = {}; }}>+ IDEA</button>
    </>
  );
}
