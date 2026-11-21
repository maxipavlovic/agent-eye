import { Fragment } from 'preact';
import { signal } from '@preact/signals';

/** Is the keys overlay open? `?` and the panel's ? button set it. */
export const keyHelp = signal(false);

/** Every key of the board and of an open session's panel, in the order the overlay lists them. */
export const KEY_LIST: ReadonlyArray<readonly [string, string]> = [
  ['R', 'Resume'],
  ['D', 'Mark done'],
  ['W', 'Move to Waiting'],
  ['P', 'Move to Paused'],
  ['Y', 'Move to Needs you'],
  ['M', 'Remind me (opens the picker)'],
  ['N', 'Continue tonight'],
  ['F', 'Fork'],
  ['C', 'Copy the resume command'],
  ['H', 'Hide'],
  ['L', 'Loose ends (scroll to LEFT BEHIND)'],
  ['E', 'Export'],
  ['X', 'Forget'],
  ['← →', 'Previous or next card'],
  ['/', 'Search (also ⌘K)'],
  ['Esc', 'Close'],
  ['?', 'This list'],
];

/** The list of keys, over the board. It closes with `?`, Esc, the scrim or CLOSE. */
export function KeyHelpHost() {
  if (!keyHelp.value) return null;
  const close = () => { keyHelp.value = false; };
  return (
    <>
      <div class="scrim" role="presentation" onClick={close} />
      <div class="modal keyhelp" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts">
        <div class="dis" />
        <h2>KEYS</h2>
        <p class="hint">In a session's panel, when no field has the focus.</p>
        <dl class="where">
          {KEY_LIST.filter(([k]) => k !== 'X' || !!document.querySelector('.page [data-key="x"]')).map(([k, what]) => (<Fragment key={k}><dt>{k}</dt><dd>{what}</dd></Fragment>))}
        </dl>
        <div class="acts end">
          <button type="button" class="btn ghost" onClick={close}>CLOSE</button>
        </div>
      </div>
    </>
  );
}
