import { useEffect } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import type { SettingsInput, SettingsState } from '../api/types';
import { ACCENTS } from '../accents';
import { LOOK_LIST, lookNamed, type LookName } from '../looks/names';
import type { Store, ThemePref } from '../store';
import { switchLook } from './switchLook';
import type { Layout } from './layout';
import { AgentSettings } from './SettingsAgent';
import { CONFIG_FILE, Part, Row, Seg, type Folds } from './SettingsParts';
import { usePanelEsc } from './panelEsc';

/** Is the SETTINGS panel open? The header's gear sets it (so does `,`). */
export const settingsPanel = signal(false);

const LAYOUTS: ReadonlyArray<readonly [Layout, string]> = [['lanes', 'LANES'], ['grid', 'GRID']];
const THEMES: ReadonlyArray<readonly [ThemePref, string]> = [['light', 'LIGHT'], ['dark', 'DARK'], ['auto', 'AUTO']];

/** The sections open when the panel opens: only how the board looks. */
const FIRST_OPEN: ReadonlySet<string> = new Set(['LOOK']);

/** The text a list of [value, text] gives `v`. */
const textOf = <T,>(list: ReadonlyArray<readonly [T, string]>, v: T) => list.find(([k]) => k === v)?.[1];

/** How the board looks: the look, the accent colour, the layout and the theme. They apply at once and live in the browser and the engine. */
function LookSettings({ store, layout, setLayout, folds }: { store: Store; layout: Layout; setLayout: (l: Layout) => void; folds: Folds }) {
  const look = lookNamed(document.documentElement.dataset.look);
  const accent = store.accent.value;
  const sum = [LOOK_LIST.find((l) => l.id === look)?.label, textOf(LAYOUTS, layout), textOf(THEMES, store.themePref.value)].filter(Boolean).join(' · ');
  return (
    <Part t="LOOK" sum={sum} folds={folds}>
      {look && (
        <Row label="Skin" help="Ink is pixel art on warm paper; Iris is a technical drawing in thin lines. The page reloads in the other one.">
          <Seg<LookName> label="Look" value={look} options={LOOK_LIST.map((l) => [l.id, l.label] as const)} onPick={(id) => void switchLook(store, id)} />
        </Row>
      )}
      <Row label="Colour" help="The accent: it marks what needs you.">
        <div class="accs" role="group" aria-label="Accent colour">
          {ACCENTS.map((a) => (
            <button key={a.id} type="button" class="acc" data-accent={a.id} aria-label={a.name} aria-pressed={accent === a.id} title={a.name} onClick={() => store.setAccent(a.id)} />
          ))}
        </div>
      </Row>
      <Row label="Layout" help="Lanes side by side, or a grid that wraps.">
        <Seg label="Layout" value={layout} options={LAYOUTS} onPick={setLayout} />
      </Row>
      <Row label="Theme" help="Auto follows your system.">
        <Seg label="Theme" value={store.themePref.value} options={THEMES} onPick={(t) => store.setTheme(t)} />
      </Row>
    </Part>
  );
}

/**
 * The SETTINGS panel: how the board looks, and the agent settings that used to live only in `config.toml`. Each section
 * folds; only LOOK is open when the panel opens, and a folded heading says what is inside.
 */
export function SettingsHost({ store, layout, setLayout }: { store: Store; layout: Layout; setLayout: (l: Layout) => void }) {
  const open = settingsPanel.value;
  const state = useSignal<SettingsState | null>(null);
  const loaded = useSignal(false);
  const saved = useSignal('');
  const opened = useSignal(FIRST_OPEN);
  useEffect(() => {
    if (!open) return;
    let live = true;
    state.value = null;
    loaded.value = false;
    saved.value = '';
    opened.value = FIRST_OPEN;
    store.loadNotice();
    void store.settings().then((s) => {
      if (!live) return;
      state.value = s;
      loaded.value = true;
    });
    return () => { live = false; };
  }, [open]);
  usePanelEsc(settingsPanel);
  if (!open) return null;
  const close = () => { settingsPanel.value = false; };
  const folds: Folds = {
    open: opened.value,
    toggle: (t) => {
      const next = new Set(opened.value);
      if (!next.delete(t)) next.add(t);
      opened.value = next;
    },
  };
  const save = async (input: SettingsInput) => {
    saved.value = '';
    const s = await store.saveSettings(input);
    if (!s) return false;
    state.value = s;
    saved.value = 'Saved';
    if (input['remind.notify'] !== undefined) store.loadNotice();
    if (Object.keys(input).some((k) => k.startsWith('clock.'))) store.loadClockHint();
    return true;
  };
  return (
    <>
      <div class="scrim" role="presentation" onClick={close} />
      <aside class="pagewrap" aria-label="Settings">
        <div class="page settings">
          <button class="x" onClick={close} aria-label="Close (Esc)">ESC ✕</button>
          <header class="dh"><div><h2>Settings</h2><p class="umeta" role="status">{saved.value || 'Changes apply at once.'}</p></div></header>
          <LookSettings store={store} layout={layout} setLayout={setLayout} folds={folds} />
          {!loaded.value && <p class="state">Loading…<i class="dots" /></p>}
          {loaded.value && !state.value && <p class="state">This engine has no agent settings. Update eye, then reload.</p>}
          {state.value && <AgentSettings store={store} state={state.value} save={save} folds={folds} />}
          <p class="umeta more">More in <code>{CONFIG_FILE}</code>: folders, prices, exclude lists and the night sandbox. See <code>docs/config.md</code>.</p>
        </div>
        <div class="dis" />
      </aside>
    </>
  );
}
