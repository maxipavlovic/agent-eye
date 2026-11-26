import { useEffect, useLayoutEffect } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import type { ColumnId, SettingsInput, SettingsState } from '../api/types';
import { ACCENTS } from '../accents';
import { LOOK_LIST, lookNamed, type LookName } from '../looks/names';
import type { Store, ThemePref } from '../store';
import { switchLook } from './AccentPicker';
import { FOLDABLE, LANE_LABEL, ORDER } from './columns';
import type { Layout, SetFold } from './layout';
import { Sec } from './Sections';
import { AgentSettings } from './SettingsAgent';
import { Row, Seg } from './SettingsParts';

/** Is the SETTINGS panel open? The header's gear sets it (so does `,`). */
export const settingsPanel = signal(false);

const LAYOUTS: ReadonlyArray<readonly [Layout, string]> = [['lanes', 'LANES'], ['grid', 'GRID']];
const THEMES: ReadonlyArray<readonly [ThemePref, string]> = [['light', 'LIGHT'], ['dark', 'DARK'], ['auto', 'AUTO']];

/** How the board looks: the look, the accent colour, the layout and the theme. They apply at once and live in the browser and the engine. */
function LookSettings({ store, layout, setLayout }: { store: Store; layout: Layout; setLayout: (l: Layout) => void }) {
  const look = lookNamed(document.documentElement.dataset.look);
  const accent = store.accent.value;
  return (
    <Sec t="LOOK">
      {look && (
        <Row label="Look" help="Ink is pixel art on warm paper; Iris is a technical drawing in thin lines. The page reloads in the other one.">
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
    </Sec>
  );
}

/** What the board remembers about its own screen: which lanes are folded. */
function BoardSettings({ folded, setFold }: { folded: ReadonlySet<ColumnId>; setFold: SetFold }) {
  return (
    <Sec t="BOARD">
      <Row label="Folded lanes" help="A folded lane shows only its name and count. Needs you never folds.">
        <div class="fchips" role="group" aria-label="Folded lanes">
          {ORDER.filter((id) => FOLDABLE.has(id)).map((id) => (
            <button key={id} type="button" aria-pressed={folded.has(id)} onClick={() => setFold(id, !folded.has(id))}>{LANE_LABEL[id].name}</button>
          ))}
        </div>
      </Row>
    </Sec>
  );
}

/** The SETTINGS panel: how the board looks, what it remembers, and the agent settings that used to live only in `config.toml`. */
export function SettingsHost({ store, layout, setLayout, folded, setFold }: { store: Store; layout: Layout; setLayout: (l: Layout) => void; folded: ReadonlySet<ColumnId>; setFold: SetFold }) {
  const open = settingsPanel.value;
  const state = useSignal<SettingsState | null>(null);
  const loaded = useSignal(false);
  const saved = useSignal('');
  useEffect(() => {
    if (!open) return;
    let live = true;
    state.value = null;
    loaded.value = false;
    saved.value = '';
    store.loadNotice();
    void store.settings().then((s) => {
      if (!live) return;
      state.value = s;
      loaded.value = true;
    });
    return () => { live = false; };
  }, [open]);
  // Esc listens from before the paint, so the first one closes this and not the panel under it
  useLayoutEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopImmediatePropagation();
      settingsPanel.value = false;
    };
    addEventListener('keydown', onKey, true);
    return () => removeEventListener('keydown', onKey, true);
  }, [open]);
  if (!open) return null;
  const close = () => { settingsPanel.value = false; };
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
          <LookSettings store={store} layout={layout} setLayout={setLayout} />
          <BoardSettings folded={folded} setFold={setFold} />
          {!loaded.value && <p class="state">Loading…<i class="dots" /></p>}
          {loaded.value && !state.value && <p class="state">This engine has no agent settings. Update eye, then reload.</p>}
          {state.value && <AgentSettings store={store} state={state.value} save={save} />}
          <p class="umeta more">More in <code>~/.agent-eye/config.toml</code>: folders, prices, exclude lists and the night sandbox. See <code>docs/config.md</code>.</p>
        </div>
        <div class="dis" />
      </aside>
    </>
  );
}
