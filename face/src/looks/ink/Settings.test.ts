// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { SettingsInput, SettingsState, Source } from '../../api/types';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { keyHelp } from '../../board/KeyHelp';
import { settingsPanel } from '../../board/Settings';
import { resetFolds } from '../../board/Sections';
import { Root } from './Root';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  globalThis.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as typeof matchMedia;
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); resetFolds(); });
afterEach(() => {
  settingsPanel.value = false;
  keyHelp.value = false;
  if (mounted) render(null, mounted);
  mounted?.remove();
  mounted = null;
  delete document.documentElement.dataset.accent;
  delete document.documentElement.dataset.theme;
});

/** An engine that keeps what is saved, like the real one. */
function engine() {
  let state: SettingsState = structuredClone(sampleState);
  const saveSettings = vi.fn(async (input: SettingsInput) => {
    for (const [k, v] of Object.entries(input)) (state.settings as Record<string, { value: unknown }>)[k]!.value = v;
    state = structuredClone(state);
    return state;
  });
  return { saveSettings, source: { ...fixtureSource(), name: 'live' as const, settings: async () => structuredClone(state), saveSettings } as Source };
}
const sampleState = await fixtureSource().settings();

async function mount(src: Source = fixtureSource()) {
  const store = createStore(src);
  const root = document.createElement('div');
  document.body.appendChild(root);
  mounted = root;
  render(h(Root, { store }), root);
  await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
  const $ = <T extends Element = HTMLElement>(sel: string) => root.querySelector<T>(sel);
  const $$ = <T extends Element = HTMLElement>(sel: string) => [...root.querySelectorAll<T>(sel)];
  const open = async () => {
    $('.hd .gear')!.click();
    await vi.waitFor(() => expect($('.settings .acts, .settings .srow')).not.toBeNull());
  };
  const press = (group: string, text: string) => $$<HTMLButtonElement>(`.settings [aria-label="${group}"] button`).find((b) => b.textContent === text)!.click();
  return { store, root, $, $$, open, press };
}

const key = (k: string) => document.body.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));

describe('the settings gear', () => {
  it('sits after USAGE and before + IDEA, and the header holds no look, colour, layout or theme control', async () => {
    const { $$, $ } = await mount();
    const names = $$('.hd > button, .hd > .accpick, .hd > .theme').map((b) => b.getAttribute('aria-label') || b.textContent);
    expect(names.slice(-3)).toEqual(['USAGE', 'Settings', '+ IDEA']);
    expect($('.hd [aria-label=Layout]')).toBeNull();
    expect($('.hd [aria-label=Theme]')).toBeNull();
    expect($('.hd .acc')).toBeNull();
  });

  it('opens the panel, names it, and closes on Esc, the scrim and ESC', async () => {
    const { $, open } = await mount();
    expect($('aside[aria-label=Settings]')).toBeNull();
    await open();
    expect($('aside[aria-label=Settings]')).not.toBeNull();
    expect($('.hd .gear')!.getAttribute('aria-pressed')).toBe('true');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await vi.waitFor(() => expect($('aside[aria-label=Settings]')).toBeNull());
    await open();
    $('.scrim')!.click();
    await vi.waitFor(() => expect($('aside[aria-label=Settings]')).toBeNull());
    await open();
    $('.settings .x')!.click();
    await vi.waitFor(() => expect($('aside[aria-label=Settings]')).toBeNull());
  });

  it('is on the , key, and the ? list names it', async () => {
    const { $ } = await mount();
    key(',');
    await vi.waitFor(() => expect($('aside[aria-label=Settings]')).not.toBeNull());
    key(',');
    await vi.waitFor(() => expect($('aside[aria-label=Settings]')).toBeNull());
    key('?');
    await vi.waitFor(() => expect($('.keyhelp')).not.toBeNull());
    expect($('.keyhelp')!.textContent).toContain('Settings');
  });

  it('leaves the , key to a field that has the focus', async () => {
    const { $ } = await mount();
    $<HTMLInputElement>('#q')!.dispatchEvent(new KeyboardEvent('keydown', { key: ',', bubbles: true }));
    expect($('aside[aria-label=Settings]')).toBeNull();
  });
});

describe('the look settings', () => {
  it('lists the eight colours by name, marks the chosen one and sets the accent', async () => {
    const { $, $$, open, store } = await mount();
    await open();
    const sw = $$<HTMLButtonElement>('.settings [aria-label="Accent colour"] .acc');
    expect(sw.map((s) => s.getAttribute('aria-label'))).toEqual(['Vermilion (original)', 'Cobalt', 'Teal', 'Moss', 'Amber', 'Magenta', 'Violet', 'Graphite']);
    expect(sw.map((s) => s.getAttribute('aria-pressed'))).toEqual(['true', ...Array(7).fill('false')]);
    sw.find((s) => s.getAttribute('aria-label') === 'Moss')!.click();
    await vi.waitFor(() => expect(document.documentElement.dataset.accent).toBe('moss'));
    expect(store.accent.value).toBe('moss');
    expect($('.settings [aria-label="Accent colour"] [aria-pressed=true]')!.getAttribute('aria-label')).toBe('Moss');
  });

  it('changes the theme and the layout, each remembered under its own key, and the panel stays open', async () => {
    const { $, open, press, store } = await mount();
    await open();
    press('Theme', 'DARK');
    await vi.waitFor(() => expect(document.documentElement.dataset.theme).toBe('dark'));
    expect(localStorage.getItem('eye.theme')).toBe('dark');
    expect(store.themePref.value).toBe('dark');
    press('Layout', 'GRID');
    await vi.waitFor(() => expect($('.board.gridview')).not.toBeNull());
    expect(localStorage.getItem('eye.ink.layout')).toBe('grid');
    expect($('aside[aria-label=Settings]')).not.toBeNull();
    press('Layout', 'LANES');
    await vi.waitFor(() => expect($('.board.lanes')).not.toBeNull());
  });

  it('shows the look choice only where the page has a look', async () => {
    const a = await mount();
    await a.open();
    expect(a.$('.settings [aria-label=Look]')).toBeNull();
    render(null, a.root);
    settingsPanel.value = false;
    document.documentElement.dataset.look = 'ink';
    const b = await mount();
    await b.open();
    expect(b.$$('.settings [aria-label=Look] button').map((x) => [x.textContent, x.getAttribute('aria-pressed')])).toEqual([['INK', 'true'], ['IRIS', 'false']]);
    delete document.documentElement.dataset.look;
  });

  it('folds and unfolds a lane from the board section, as the lane rail does', async () => {
    const { $, $$, open } = await mount();
    await open();
    const chips = $$<HTMLButtonElement>('.settings [aria-label="Folded lanes"] button');
    expect(chips.map((c) => c.textContent)).toEqual(['IDEAS', 'RUNNING', 'WAITING', 'PAUSED', 'DONE']);
    expect(chips.map((c) => c.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'false', 'false', 'true']);
    chips[0]!.click();
    await vi.waitFor(() => expect($('.rail.folded.c-ideas')).not.toBeNull());
    expect(JSON.parse(localStorage.getItem('eye.ink.fold')!)).toContain('ideas');
  });
});

describe('the agent settings', () => {
  it('shows the sample board values and says that nothing is saved there', async () => {
    const { $, $$, open } = await mount();
    await open();
    await vi.waitFor(() => expect($('.settings [aria-label=Agents]')).not.toBeNull());
    expect($('.settings')!.textContent).toContain('sample board');
    expect($$<HTMLButtonElement>('.settings [aria-label=Summaries] button').every((b) => b.disabled)).toBe(true);
    expect($$<HTMLButtonElement>('.settings [aria-label=Agents] button').every((b) => b.disabled)).toBe(true);
  });

  it('names each setting with one plain line, and points to eye config for the rest', async () => {
    const { $, $$, open } = await mount(engine().source);
    await open();
    await vi.waitFor(() => expect($('.settings [aria-label=Agents]')).not.toBeNull());
    expect($$('.settings .sec h4').map((h4) => h4.textContent)).toEqual(['LOOK', 'BOARD', 'SUMMARIES', 'NIGHT SHIFT', 'REMINDERS', 'AGENTS']);
    expect($('.settings')!.textContent).toContain('Takes effect when the background service restarts');
    expect($('.settings .more')!.textContent).toContain('eye config');
    expect($('.settings .more')!.textContent).toContain('docs/config.md');
  });

  it('saves a switch at once and says Saved', async () => {
    const { saveSettings, source } = engine();
    const { $, open, press } = await mount(source);
    await open();
    await vi.waitFor(() => expect($('.settings [aria-label=Summaries]')).not.toBeNull());
    press('Summaries', 'OFF');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'summary.enabled': false }));
    await vi.waitFor(() => expect($('.settings [role=status]')!.textContent).toBe('Saved'));
    expect($('.settings [aria-label=Summaries] [aria-pressed=true]')!.textContent).toBe('OFF');
    press('Work in', 'COPY');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'night.isolation': 'copy' }));
    press('Night shift', 'ON');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'night.enabled': true }));
  });

  it('saves the window and the model from their fields, and only when they changed', async () => {
    const { saveSettings, source } = engine();
    const { $, open } = await mount(source);
    await open();
    await vi.waitFor(() => expect($('.settings form.nset')).not.toBeNull());
    const [model, win] = [...document.querySelectorAll<HTMLInputElement>('.settings form.nset input')];
    const submit = (input: HTMLInputElement, v: string) => {
      input.value = v;
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    };
    submit(win!, '01:00-07:00');
    submit(model!, 'sonnet');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'summary.model': 'sonnet' }));
    submit(win!, '23:00-05:00');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'night.window': '23:00-05:00' }));
    expect(saveSettings).toHaveBeenCalledTimes(2);
  });

  it('keeps the field as it was and says why when the engine refuses a value', async () => {
    const { source } = engine();
    const saveSettings = vi.fn(async () => { throw new Error("'7am' is not a window: write it as 01:00-07:00"); });
    const { $, open } = await mount({ ...source, saveSettings });
    await open();
    await vi.waitFor(() => expect($('.settings form.nset')).not.toBeNull());
    const win = document.querySelectorAll<HTMLInputElement>('.settings form.nset input')[1]!;
    win.value = '7am';
    win.dispatchEvent(new Event('input', { bubbles: true }));
    win.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect($('.toasts')!.textContent).toContain('is not a window'));
    await vi.waitFor(() => expect(win.value).toBe('01:00-07:00'));
    expect($('.settings [role=status]')!.textContent).not.toBe('Saved');
  });

  it('turns an agent on or off by sending the whole list', async () => {
    const { saveSettings, source } = engine();
    const { $, open } = await mount(source);
    await open();
    await vi.waitFor(() => expect($('.settings [aria-label=Agents]')).not.toBeNull());
    const chips = [...document.querySelectorAll<HTMLButtonElement>('.settings [aria-label=Agents] button')];
    expect(chips.map((c) => [c.textContent, c.getAttribute('aria-pressed')])).toEqual([['Claude Code', 'true'], ['Codex CLI', 'false']]);
    chips[1]!.click();
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'agents.enabled': ['claude', 'codex'] }));
    await vi.waitFor(() => expect([...document.querySelectorAll('.settings [aria-label=Agents] button')].map((c) => c.getAttribute('aria-pressed'))).toEqual(['true', 'true']));
    chips[0]!.click();
    await vi.waitFor(() => expect(saveSettings).toHaveBeenLastCalledWith({ 'agents.enabled': ['codex'] }));
  });

  it('says so when the engine is too old to have settings', async () => {
    const { $, open } = await mount({ ...fixtureSource(), name: 'live', settings: async () => { throw new Error('HTTP 404'); } });
    await open();
    await vi.waitFor(() => expect($('.settings .state')!.textContent).toContain('Update eye'));
    expect($('.settings [aria-label=Layout]')).not.toBeNull();
  });
});
