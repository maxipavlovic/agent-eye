import type { ComponentChildren } from 'preact';

// The two controls every setting is built from, in the panel's look-neutral markup.

/** One setting: its name, the control, and one plain line on what it does. */
export function Row({ label, help, children }: { label: string; help?: ComponentChildren; children: ComponentChildren }) {
  return (
    <div class="srow">
      <div class="nrow"><span>{label.toUpperCase()}</span>{children}</div>
      {help && <p class="umeta">{help}</p>}
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
