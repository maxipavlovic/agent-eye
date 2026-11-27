import type { ComponentChildren } from 'preact';
import { FoldSec } from './Sections';

// The parts every setting is built from, in the panel's look-neutral markup.

/** Where the settings live, as the panel names the file. */
export const CONFIG_FILE = '~/.agent-eye/config.toml';

/**
 * The sentence a setting gets when the engine marks its key `restart: true`: written here once, never by hand per row.
 * `who` names what waits for the restart when another part takes the setting at once (summaries, while judging does not).
 */
export const restartNote = (restart?: boolean, who?: string) => (restart ? ` ${who ? `${who} take it` : 'Takes effect'} when the background service restarts.` : '');

/** One setting: its name, the control, and one plain line on what it does, with the restart sentence when `restart` is set. */
export function Row({ label, help, restart, children }: { label: string; help: string; restart?: boolean; children: ComponentChildren }) {
  return (
    <div class="srow">
      <div class="nrow"><span>{label.toUpperCase()}</span>{children}</div>
      <p class="umeta">{help}{restartNote(restart)}</p>
    </div>
  );
}

/**
 * A setting the panel shows but leaves to the file: its name and value, one plain line on what it does, and where to
 * change it, with `eye restart` when the engine marks the key `restart: true`. `children` go between the two lines.
 */
export function Shown({ label, value, help, file, restart, children }: {
  label: string; value: string; help: string; file: string; restart?: boolean; children?: ComponentChildren;
}) {
  return (
    <div class="srow">
      <div class="nrow"><span>{label.toUpperCase()}</span><b class="sval">{value}</b></div>
      {children}
      <p class="umeta">{help} To change it, set <code>{file}</code> in <code>{CONFIG_FILE}</code>{restart && <>, then run <code>eye restart</code></>}.</p>
    </div>
  );
}

const ON_OFF: ReadonlyArray<readonly ['on' | 'off', string]> = [['on', 'ON'], ['off', 'OFF']];

/** ON or OFF for one on/off key. */
export function Switch({ label, on, disabled, onPick }: { label: string; on: boolean; disabled: boolean; onPick: (on: boolean) => void }) {
  return <Seg label={label} value={on ? 'on' : 'off'} options={ON_OFF} disabled={disabled} onPick={(v) => onPick(v === 'on')} />;
}

/** A segmented choice: one button per option, the current one pressed. */
export function Seg<T extends string>({ label, value, options, onPick, disabled }: {
  label: string; value: T; options: ReadonlyArray<readonly [T, string]>; onPick: (v: T) => void; disabled?: boolean;
}) {
  return (
    <div class="theme seg" role="group" aria-label={label}>
      {options.map(([k, text]) => (
        <button key={k} type="button" class={value === k ? 'on' : ''} aria-pressed={value === k} disabled={disabled} onClick={() => { if (value !== k) onPick(k); }}>{text}</button>
      ))}
    </div>
  );
}

/** The panel's open sections, by title, and the switch for one. */
export interface Folds { open: ReadonlySet<string>; toggle: (t: string) => void }

/** A section of the panel: its heading opens and folds it, and folded it shows `sum`, a short status of what is inside. */
export function Part({ t, sum, folds, children }: { t: string; sum: string; folds: Folds; children: ComponentChildren }) {
  const open = folds.open.has(t);
  return <FoldSec t={t} sum={open ? undefined : sum} open={open} toggle={() => folds.toggle(t)}>{children}</FoldSec>;
}

/** ON or OFF, as a folded heading says it. */
export const onOff = (on: boolean) => (on ? 'ON' : 'OFF');
