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
  /** A section's heading button, by its title. */
  const head = (t: string) => $$<HTMLButtonElement>('.settings .fhead').find((b) => b.firstChild?.textContent === t);
  /** Every heading as [title, aria-expanded, the folded status or null]. */
  const heads = () => $$<HTMLButtonElement>('.settings .fhead').map((b) => [b.firstChild?.textContent, b.getAttribute('aria-expanded'), b.querySelector('.fsum')?.textContent ?? null]);
  /** Open these sections, once the engine's settings are in. */
  const unfold = async (...titles: string[]) => {
    for (const t of titles) {
      await vi.waitFor(() => expect(head(t)).toBeDefined());
      if (head(t)!.getAttribute('aria-expanded') !== 'true') head(t)!.click();
      await vi.waitFor(() => expect(head(t)!.getAttribute('aria-expanded')).toBe('true'));
    }
  };
  return { store, root, $, $$, open, press, head, heads, unfold };
}

const AGENT_PARTS = ['SUMMARIES', 'STARTED BY PROGRAMS', 'NIGHT SHIFT', 'REMINDERS', 'HOURS', 'AGENTS'];
/** A section of the open panel, by its title. */
const section = (t: string) => [...document.querySelectorAll('.settings .sec')].find((sec) => sec.querySelector('.fhead')?.firstChild?.textContent === t)!;
const RESTART = /when the background service restarts|then run eye restart/g;

const key = (k: string) => document.body.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));

describe('the settings gear', () => {
  it('sits after USAGE and HOURS and before + IDEA, and the header holds no look, colour, layout or theme control', async () => {
    const { $$, $ } = await mount();
    const names = $$('.hd > button, .hd > .accpick, .hd > .theme').map((b) => b.getAttribute('aria-label') || b.textContent);
    expect(names.slice(-4)).toEqual(['USAGE', 'HOURS', 'Settings', '+ IDEA']);
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

  it('folds to one line with what it is set to, and has no BOARD section: lanes fold on the board itself', async () => {
    const { $, head, heads, open } = await mount();
    await open();
    await vi.waitFor(() => expect(head('AGENTS')).toBeDefined());
    head('LOOK')!.click();
    await vi.waitFor(() => expect(head('LOOK')!.getAttribute('aria-expanded')).toBe('false'));
    expect(heads()[0]).toEqual(['LOOK', 'false', 'LANES · AUTO']);
    expect($('.settings [aria-label=Layout]')).toBeNull();
    expect(heads().map(([t]) => t)).not.toContain('BOARD');
    expect($('.settings [aria-label="Folded lanes"]')).toBeNull();
  });
});

describe('the folded sections', () => {
  it('opens with only LOOK unfolded, and each folded heading says what it is set to', async () => {
    const { $, heads, open } = await mount(engine().source);
    await open();
    await vi.waitFor(() => expect(heads()).toHaveLength(7));
    expect(heads()).toEqual([
      ['LOOK', 'true', null],
      ['SUMMARIES', 'false', 'ON · haiku'],
      ['STARTED BY PROGRAMS', 'false', '8 FOUND · OFF'],
      ['NIGHT SHIFT', 'false', 'OFF'],
      ['REMINDERS', 'false', 'OFF'],
      ['HOURS', 'false', 'ON'],
      ['AGENTS', 'false', 'Claude Code'],
    ]);
    expect($('.settings [aria-label=Theme]')).not.toBeNull();
    expect($('.settings [aria-label=Summaries]')).toBeNull();
    // the headings are the switches, for the keyboard too
    for (const b of document.querySelectorAll('.settings h4 > .fhead')) expect(b.tagName).toBe('BUTTON');
  });

  it('opens and folds a section from its heading, and the next opening starts from LOOK alone again', async () => {
    const { $, head, heads, open, unfold } = await mount(engine().source);
    await open();
    await unfold('HOURS');
    expect($('.settings [aria-label="Judge corrections"]')).not.toBeNull();
    expect(heads().find(([t]) => t === 'HOURS')).toEqual(['HOURS', 'true', null]);
    head('HOURS')!.click();
    await vi.waitFor(() => expect($('.settings [aria-label="Judge corrections"]')).toBeNull());
    await unfold('AGENTS');
    key(',');
    await vi.waitFor(() => expect($('aside[aria-label=Settings]')).toBeNull());
    await open();
    await vi.waitFor(() => expect(heads()).toHaveLength(7));
    expect(heads().map(([, o]) => o)).toEqual(['true', 'false', 'false', 'false', 'false', 'false', 'false']);
  });

  it('says what the engine has in each folded heading: summaries off, the night window, the agents in order', async () => {
    const state = structuredClone(sampleState);
    state.settings['summary.enabled'].value = false;
    state.settings['night.enabled'].value = true;
    state.settings['remind.notify'].value = true;
    state.settings['clock.enabled'].value = false;
    state.settings['agents.enabled'].value = ['codex', 'claude'];
    state.settings['board.quiet'].value = ['app'];
    const { heads, open } = await mount({ ...fixtureSource(), name: 'live' as const, settings: async () => structuredClone(state) } as Source);
    await open();
    await vi.waitFor(() => expect(heads()).toHaveLength(7));
    expect(heads().slice(1).map(([, , sum]) => sum)).toEqual(['OFF', 'APP QUIET', 'ON · 01:00-07:00', 'ON', 'OFF', 'Codex CLI, Claude Code']);
  });

  it('closes on Esc with the focus on a heading', async () => {
    const { $, head, open } = await mount(engine().source);
    await open();
    await vi.waitFor(() => expect(head('HOURS')).toBeDefined());
    head('HOURS')!.focus();
    head('HOURS')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await vi.waitFor(() => expect($('aside[aria-label=Settings]')).toBeNull());
  });
});

describe('the agent settings', () => {
  it('shows the sample board values and says that nothing is saved there', async () => {
    const { $, $$, open, unfold } = await mount();
    await open();
    await unfold(...AGENT_PARTS);
    expect($('.settings')!.textContent).toContain('sample board');
    expect($$<HTMLButtonElement>('.settings [aria-label=Summaries] button').every((b) => b.disabled)).toBe(true);
    expect($<HTMLSelectElement>('.settings select')!.disabled).toBe(true);
  });

  it('names each setting with one plain line, and points to config.toml for the rest', async () => {
    const { $, heads, open, unfold } = await mount(engine().source);
    await open();
    await unfold(...AGENT_PARTS);
    expect(heads().map(([t]) => t)).toEqual(['LOOK', ...AGENT_PARTS]);
    expect($('.settings')!.textContent).toContain('Takes effect when the background service restarts');
    expect($('.settings .more')!.textContent).toContain('config.toml');
    expect($('.settings .more')!.textContent).toContain('docs/config.md');
  });

  it('shows the steering clock as ON or OFF, not as a switch, and says to change it in the file and restart', async () => {
    const { $, open, unfold } = await mount(engine().source);
    await open();
    await unfold('HOURS');
    expect($('.settings [aria-label="Steering clock"]')).toBeNull();
    const row = [...document.querySelectorAll('.settings .srow')].find((r) => r.textContent?.includes('Notes when you prompt'))!;
    expect(row.querySelector('.nrow')!.textContent).toBe('STEERING CLOCKON');
    expect(row.querySelector('button')).toBeNull();
    expect(row.textContent).toContain('To change it, set [clock] enabled in ~/.agent-eye/config.toml, then run eye restart.');
  });

  it('lists the agents eye follows, the ones it does not quieter, and says to change them in the file and restart', async () => {
    const { $, open, unfold } = await mount(engine().source);
    await open();
    await unfold('AGENTS');
    const sec = section('AGENTS');
    expect(sec.querySelector('.sval')!.textContent).toBe('Claude Code');
    expect(sec.querySelector('.soff')!.textContent).toBe('Not followed: Codex CLI.');
    expect(sec.querySelector('button:not(.fhead)')).toBeNull();
    expect(sec.textContent).toContain('To change it, set [agents] enabled in ~/.agent-eye/config.toml, then run eye restart.');
    expect($('.settings [aria-label=Agents]')).toBeNull();
  });

  it('writes the restart sentence once for every setting the engine marks as needing a restart', async () => {
    const state = structuredClone(sampleState);
    for (const setting of Object.values(state.settings)) (setting as { restart?: boolean }).restart = true;
    const { $$, open, unfold } = await mount({ ...fixtureSource(), name: 'live' as const, settings: async () => structuredClone(state) } as Source);
    await open();
    await unfold(...AGENT_PARTS);
    const said = (el: Element) => (el.textContent ?? '').match(RESTART)?.length ?? 0;
    // each row and each form beside it, one sentence apiece; a row shown but left to the file says to run eye restart
    const rows = AGENT_PARTS.flatMap((t) => [...section(t).querySelectorAll('.srow, .nset')]);
    expect(rows.length).toBeGreaterThan(9);
    for (const row of rows) expect(said(row), row.textContent!).toBe(1);
  });

  it('writes no restart sentence where the engine does not mark one, and the rows left to the file still say where they are set', async () => {
    const state = structuredClone(sampleState);
    for (const setting of Object.values(state.settings)) delete (setting as { restart?: boolean }).restart;
    const { open, unfold } = await mount({ ...fixtureSource(), name: 'live' as const, settings: async () => structuredClone(state) } as Source);
    await open();
    await unfold(...AGENT_PARTS);
    expect(document.querySelector('.settings')!.textContent!.match(RESTART)).toBeNull();
    expect(section('AGENTS').textContent).toContain('To change it, set [agents] enabled in ~/.agent-eye/config.toml.');
  });

  it('saves a switch at once and says Saved', async () => {
    const { saveSettings, source } = engine();
    const { $, open, press, unfold } = await mount(source);
    await open();
    await unfold('SUMMARIES', 'NIGHT SHIFT');
    press('Summaries', 'OFF');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'summary.enabled': false }));
    await vi.waitFor(() => expect($('.settings [role=status]')!.textContent).toBe('Saved'));
    expect($('.settings [aria-label=Summaries] [aria-pressed=true]')!.textContent).toBe('OFF');
    press('Work in', 'COPY');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'night.isolation': 'copy' }));
    press('Night shift', 'ON');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'night.enabled': true }));
  });

  it('saves the steering clock keys: the switches at once, the break from its field and only inside its range', async () => {
    const { saveSettings, source } = engine();
    const { $, $$, open, press, unfold } = await mount(source);
    await open();
    await unfold('HOURS');
    press('Hint', 'ON');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'clock.hints': true }));
    press('Judge corrections', 'OFF');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'clock.llm': false }));
    press('Idle time', 'ON');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'clock.idle': true }));
    expect($('.settings')!.textContent).toContain('Only a number of seconds is read; never what you type.');
    const input = $$<HTMLInputElement>('.settings form.nset input').find((i) => i.type === 'number')!;
    expect([input.min, input.max, input.value]).toEqual(['10', '240', '45']);
    input.value = '30';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'clock.break_minutes': 30 }));
    saveSettings.mockClear();
    input.value = '5.5';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(input.value).toBe('30'));
    expect(saveSettings).not.toHaveBeenCalled();
  });

  it('keeps the clock-only settings off while the clock is off', async () => {
    const off = structuredClone(sampleState);
    off.settings['clock.enabled'].value = false;
    const { $, $$, open, unfold } = await mount({ ...fixtureSource(), name: 'live' as const, settings: async () => structuredClone(off) } as Source);
    await open();
    await unfold('HOURS');
    for (const group of ['Judge corrections', 'Hint', 'Idle time']) {
      expect($$<HTMLButtonElement>(`.settings [aria-label="${group}"] button`).every((b) => b.disabled), group).toBe(true);
    }
    expect($<HTMLInputElement>('.settings input[type=number]')!.disabled).toBe(true);
    expect(section('HOURS').querySelector('.sval')!.textContent).toBe('OFF');
  });

  it('says when idle time is not available on this system, and leaves the switch off', async () => {
    const noIdle = structuredClone(sampleState)!;
    noIdle.settings['clock.idle'] = { type: 'bool', value: false, available: false };
    const { $, open, unfold } = await mount({ ...fixtureSource(), name: 'live', settings: async () => structuredClone(noIdle) } as Source);
    await open();
    await unfold('HOURS');
    await vi.waitFor(() => expect($('.settings')!.textContent).toContain('Not available on this system.'));
    expect($('.settings [aria-label="Idle time"] button')!.hasAttribute('disabled')).toBe(true);
  });

  it('saves the window from its field, and only when it changed', async () => {
    const { saveSettings, source } = engine();
    const { $, open, unfold } = await mount(source);
    await open();
    await unfold('NIGHT SHIFT');
    const win = $<HTMLInputElement>('.settings input[placeholder="01:00-07:00"]')!;
    const submit = (v: string) => {
      win.value = v;
      win.dispatchEvent(new Event('input', { bubbles: true }));
      win.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    };
    submit('01:00-07:00');
    submit('23:00-05:00');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'night.window': '23:00-05:00' }));
    expect(saveSettings).toHaveBeenCalledTimes(1);
  });

  it('keeps the field as it was and says why when the engine refuses a value', async () => {
    const { source } = engine();
    const saveSettings = vi.fn(async () => { throw new Error("'7am' is not a window: write it as 01:00-07:00"); });
    const { $, open, unfold } = await mount({ ...source, saveSettings });
    await open();
    await unfold('NIGHT SHIFT');
    const win = $<HTMLInputElement>('.settings input[placeholder="01:00-07:00"]')!;
    win.value = '7am';
    win.dispatchEvent(new Event('input', { bubbles: true }));
    win.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect($('.toasts')!.textContent).toContain('is not a window'));
    await vi.waitFor(() => expect(win.value).toBe('01:00-07:00'));
    expect($('.settings [role=status]')!.textContent).not.toBe('Saved');
  });

  it('says so when the engine is too old to have settings', async () => {
    const { $, open } = await mount({ ...fixtureSource(), name: 'live', settings: async () => { throw new Error('HTTP 404'); } });
    await open();
    await vi.waitFor(() => expect($('.settings .state')!.textContent).toContain('Update eye'));
    expect($('.settings [aria-label=Layout]')).not.toBeNull();
  });
});

describe('the sessions started by programs', () => {
  const T = 'STARTED BY PROGRAMS';
  const withQuiet = (q: SettingsState['settings']['board.quiet']) => {
    const state = structuredClone(sampleState);
    state.settings['board.quiet'] = q;
    return { ...fixtureSource(), name: 'live' as const, settings: async () => structuredClone(state) } as Source;
  };
  const switches = () => [...section(T).querySelectorAll('[role=group]')].map((g) => [g.getAttribute('aria-label'), g.querySelector('[aria-pressed=true]')?.textContent]);

  it('comes after SUMMARIES with a switch for each program origin found, its sessions and how many quiet takes out of Needs you', async () => {
    const { heads, open, unfold } = await mount(engine().source);
    await open();
    await unfold(T);
    expect(heads().map(([t]) => t).slice(1, 3)).toEqual(['SUMMARIES', T]);
    expect(switches()).toEqual([['Quiet App', 'OFF'], ['Quiet Headless', 'OFF']]);
    const lines = [...section(T).querySelectorAll('.srow .umeta')].map((p) => p.textContent);
    expect(lines).toEqual([
      "5 sessions started by an app through an agent's SDK; quiet would take 2 out of Needs you.",
      '3 sessions started by a script or a CI job; quiet would take none out of Needs you.',
    ]);
    expect(section(T).textContent).toContain('A quiet session is Running while it works and in the archive otherwise, never in Needs you by itself; it gets no summary and stays out of HOURS.');
  });

  it('saves the list at once, folds to which are quiet, and loads the board again', async () => {
    const { saveSettings, source } = engine();
    const board = vi.spyOn(source, 'board');
    const { $, head, heads, open, press, unfold } = await mount(source);
    await open();
    await unfold(T);
    const loads = board.mock.calls.length;
    press('Quiet App', 'ON');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'board.quiet': ['app'] }));
    await vi.waitFor(() => expect($('.settings [role=status]')!.textContent).toBe('Saved'));
    await vi.waitFor(() => expect(board.mock.calls.length).toBeGreaterThan(loads));
    expect(switches()).toEqual([['Quiet App', 'ON'], ['Quiet Headless', 'OFF']]);
    expect(section(T).querySelector('.srow .umeta')!.textContent).toBe("5 sessions started by an app through an agent's SDK; 2 kept out of Needs you.");
    press('Quiet Headless', 'ON');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenLastCalledWith({ 'board.quiet': ['app', 'headless'] }));
    head(T)!.click();
    await vi.waitFor(() => expect(heads()[2]).toEqual([T, 'false', 'QUIET']));
    await unfold(T);
    press('Quiet App', 'OFF');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenLastCalledWith({ 'board.quiet': ['headless'] }));
  });

  it('says so when no session was started by a program, and shows no switch', async () => {
    const { heads, open, unfold } = await mount(withQuiet({ type: 'origins', choices: [], value: [] }));
    await open();
    await unfold(T);
    expect(section(T).textContent).toContain('No sessions started by a program on this machine.');
    expect(switches()).toEqual([]);
    expect(heads()[2]).toEqual([T, 'true', null]);
  });

  it('keeps the switch of an origin set to quiet when none of its sessions are left, so it can be turned off', async () => {
    const q = { type: 'origins' as const, choices: [{ id: 'headless', name: 'Headless', about: 'started by a script or a CI job', sessions: 0, needs_you: 0 }], value: ['headless'] };
    const { heads, open, unfold } = await mount(withQuiet(q));
    await open();
    await vi.waitFor(() => expect(heads()[2]).toEqual([T, 'false', 'QUIET']));
    await unfold(T);
    expect(switches()).toEqual([['Quiet Headless', 'ON']]);
    expect(section(T).querySelector('.srow .umeta')!.textContent).toBe('No sessions started by a script or a CI job left.');
  });

  it('folds to NONE when nothing was found, and the sample board shows its switches without saving', async () => {
    const a = await mount(withQuiet({ type: 'origins', choices: [], value: [] }));
    await a.open();
    await vi.waitFor(() => expect(a.heads()[2]).toEqual([T, 'false', 'NONE']));
    render(null, a.root);
    settingsPanel.value = false;
    const b = await mount();
    await b.open();
    await b.unfold(T);
    expect(b.$$<HTMLButtonElement>('.settings [aria-label="Quiet App"] button').every((x) => x.disabled)).toBe(true);
  });
});

describe('the model for summaries', () => {
  const pickModel = (v: string) => {
    const select = document.querySelector<HTMLSelectElement>('.settings select')!;
    select.value = v;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const options = () => [...document.querySelectorAll<HTMLOptionElement>('.settings select option')].map((o) => o.textContent);

  it("is a choice of the summary agent's models, the default first, saved at once, and the help names the agent", async () => {
    const { saveSettings, source } = engine();
    const { $, head, heads, open, unfold } = await mount(source);
    await open();
    await unfold('SUMMARIES');
    expect(options()).toEqual(['Default: haiku', 'sonnet', 'opus', 'Other…']);
    expect($<HTMLSelectElement>('.settings select')!.value).toBe('');
    expect($('.settings form.nset .hint')!.textContent).toBe('The model Claude Code writes summaries with, and judges corrections with from the next nightly pass. Small is fine: a summary is short. Summaries take it when the background service restarts.');
    expect($('.settings input')).toBeNull();
    pickModel('sonnet');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'summary.model': 'sonnet' }));
    head('SUMMARIES')!.click();
    await vi.waitFor(() => expect(heads()[1]).toEqual(['SUMMARIES', 'false', 'ON · sonnet']));
    await unfold('SUMMARIES');
    pickModel('');
    await vi.waitFor(() => expect(saveSettings).toHaveBeenLastCalledWith({ 'summary.model': '' }));
  });

  it('shows a field for any other name on Other…, and SET saves it and puts it in the choice', async () => {
    const { saveSettings, source } = engine();
    const { $, open, unfold } = await mount(source);
    await open();
    await unfold('SUMMARIES');
    pickModel(' other');
    await vi.waitFor(() => expect($('.settings form.nset input')).not.toBeNull());
    const field = $<HTMLInputElement>('.settings form.nset input')!;
    await vi.waitFor(() => expect(document.activeElement).toBe(field));
    expect(saveSettings).not.toHaveBeenCalled();
    field.value = 'claude-small-2';
    field.dispatchEvent(new Event('input', { bubbles: true }));
    field.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ 'summary.model': 'claude-small-2' }));
    await vi.waitFor(() => expect($('.settings form.nset input')).toBeNull());
    expect(options()).toEqual(['Default: haiku', 'sonnet', 'opus', 'claude-small-2', 'Other…']);
    expect($<HTMLSelectElement>('.settings select')!.value).toBe('claude-small-2');
    // Other… again starts from that name, to edit it
    pickModel(' other');
    await vi.waitFor(() => expect($<HTMLInputElement>('.settings form.nset input')!.value).toBe('claude-small-2'));
  });

  it('puts the choice back and says why when the engine refuses a model', async () => {
    const { source } = engine();
    const saveSettings = vi.fn(async () => { throw new Error('summary.model: a model name is up to 64 letters'); });
    const { $, open, unfold } = await mount({ ...source, saveSettings });
    await open();
    await unfold('SUMMARIES');
    pickModel('opus');
    await vi.waitFor(() => expect($('.toasts')!.textContent).toContain('a model name is up to 64 letters'));
    await vi.waitFor(() => expect($<HTMLSelectElement>('.settings select')!.value).toBe(''));
  });

  it('offers the default and Other… from an engine that sends no model names, and names no agent', async () => {
    const old = structuredClone(sampleState);
    old.settings['summary.model'] = { type: 'text', max: 64, value: 'mine', restart: true };
    const { $, open, unfold } = await mount({ ...fixtureSource(), name: 'live' as const, settings: async () => structuredClone(old) } as Source);
    await open();
    await unfold('SUMMARIES');
    expect(options()).toEqual(['Default', 'mine', 'Other…']);
    expect($<HTMLSelectElement>('.settings select')!.value).toBe('mine');
    expect($('.settings form.nset .hint')!.textContent).toContain('The model your agent writes summaries');
  });
});
