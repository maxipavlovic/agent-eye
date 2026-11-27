import type { ComponentChildren } from 'preact';

// The two controls every setting is built from, in the panel's look-neutral markup.

/** The sentence a setting gets when the engine marks its key `restart: true`: written here once, never by hand per row. */
export const restartNote = (restart?: boolean) => (restart ? ' Takes effect when the background service restarts.' : '');

/** One setting: its name, the control, and one plain line on what it does, with the restart sentence when `restart` is set. */
export function Row({ label, help, restart, children }: { label: string; help: string; restart?: boolean; children: ComponentChildren }) {
  return (
    <div class="srow">
      <div class="nrow"><span>{label.toUpperCase()}</span>{children}</div>
      <p class="umeta">{help}{restartNote(restart)}</p>
    </div>
  );
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
