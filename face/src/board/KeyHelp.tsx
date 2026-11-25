import { Fragment } from 'preact';
import { signal } from '@preact/signals';
import type { Store } from '../store';

/** Is the keys overlay open? `?` and the panel's ? button set it. */
export const keyHelp = signal(false);

/** Every key of the board and of an open session's panel, in the order the overlay lists them. */
export const KEY_LIST: ReadonlyArray<readonly [string, string]> = [
  ['R', 'Resume'],
  ['D', 'Mark done'],
  ['W', 'Move to Waiting'],
  ['P', 'Move to Paused'],
  ['N', 'Move to Needs you'],
  ['M', 'Remind me (opens the picker)'],
  ['T', 'Continue tonight'],
  ['F', 'Fork'],
  ['C', 'Copy the resume command'],
  ['H', 'Hide'],
  ['L', 'Leftovers (open the list)'],
  ['E', 'Export'],
  ['X', 'Forget'],
  ['← →', 'Previous or next card'],
  ['/', 'Search (also ⌘K)'],
  ['Esc', 'Close'],
  ['?', 'This list'],
];

/** The keys of an open idea's panel, listed instead while one is open. */
export const IDEA_KEY_LIST: ReadonlyArray<readonly [string, string]> = [
  ['S', 'Start'],
  ['B', 'Start in the background'],
  ['T', 'Plan for tonight'],
  ['E', 'Edit'],
  ['M', 'Remind me (opens the picker)'],
  ['← →', 'Previous or next card'],
  ['/', 'Search (also ⌘K)'],
  ['Esc', 'Close'],
  ['?', 'This list'],
];

/** The list of keys, over the board. It closes with `?`, Esc, the scrim or CLOSE. */
export function KeyHelpHost({ store }: { store: Store }) {
  if (!keyHelp.value) return null;
  const close = () => { keyHelp.value = false; };
  const open = store.selected.value ? store.detail.value : null;
  const idea = open?.kind === 'idea';
  return (
    <>
      <div class="scrim" role="presentation" onClick={close} />
      <div class="modal keyhelp" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts">
        <div class="dis" />
        <h2>KEYS</h2>
        <div class="mbody" tabIndex={0} role="region" aria-label="Keys">
          <p class="hint">{idea ? "In an idea's panel" : "In a session's panel"}, when no field has the focus.</p>
          <dl class="where">
            {/* X forgets a session: listed while a session's panel is open */}
            {(idea ? IDEA_KEY_LIST : KEY_LIST.filter(([k]) => k !== 'X' || !!open)).map(([k, what]) => (<Fragment key={k}><dt>{k}</dt><dd>{what}</dd></Fragment>))}
          </dl>
        </div>
        <div class="acts end">
          <button type="button" class="btn ghost" onClick={close}>CLOSE</button>
        </div>
      </div>
    </>
  );
}
