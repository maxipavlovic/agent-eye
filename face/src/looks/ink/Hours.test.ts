// @vitest-environment happy-dom
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Source } from '../../api/types';
import { demoClock, emptyClock } from '../../data/clockSample';
import { hintClock, tooFewClock } from '../../data/clockStates';
import { demoUsage } from '../../data/usageSample';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { hoursPanel } from '../../board/Hours';
import { usagePanel } from '../../board/Usage';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  globalThis.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as typeof matchMedia;
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});

afterEach(() => {
  hoursPanel.value = false;
  usagePanel.value = false;
  if (mounted) render(null, mounted);
  mounted?.remove();
  mounted = null;
});

async function mount(over: Partial<Source>) {
  const store = createStore({ ...fixtureSource(), ...over });
  const root = document.createElement('div');
  document.body.appendChild(root);
  mounted = root;
  render(h(Root, { store }), root);
  await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
  const header = (name: string) => [...root.querySelectorAll('header.hd button')].find((b) => b.textContent === name) as HTMLButtonElement;
  return { root, header, store };
}

const hoursText = (root: HTMLElement) => root.querySelector('.hours')!.textContent!;

describe('the HOURS panel', () => {
  it('opens from the button after USAGE, says the verdict and draws the three strips and the break chart', async () => {
    const clock = vi.fn(async () => demoClock(4));
    const { root, header } = await mount({ clock });
    expect(root.querySelector('.hours')).toBeNull();
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    expect(clock).toHaveBeenLastCalledWith(4);
    expect(root.querySelector('.hours .verdict')!.textContent).toContain('Mostly you.');
    expect(hoursText(root)).toContain('12 prompts not judged yet');
    const svg = root.querySelector('.cdiagram')!;
    for (const t of ['YOU', 'THE MODEL', 'TOOL ERRORS', 'your evening']) expect(svg.textContent).toContain(t);
    // a bad band from 18 to midnight is one piece behind each of the three strips
    expect(svg.querySelectorAll('.cband')).toHaveLength(3);
    // 24 hours, three bars each at most; the hours with under 20 prompts are faint and carry a dotted cap
    expect(svg.querySelectorAll('.cbar').length).toBeGreaterThan(40);
    expect(svg.querySelectorAll('.cbar.faint').length).toBeGreaterThan(0);
    expect(svg.querySelectorAll('.ccap')).toHaveLength(svg.querySelectorAll('.cbar.faint').length);
    expect(root.querySelectorAll('.cbreak .chit')).toHaveLength(4);
    expect(root.querySelector('.hours .ctable')).toBeNull();
  });

  it('asks for another number of weeks', async () => {
    const clock = vi.fn(async () => demoClock(4));
    const { root, header } = await mount({ clock });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    ([...root.querySelectorAll('.hours .acts button')].find((b) => b.textContent === '8 WEEKS') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(clock).toHaveBeenLastCalledWith(8));
  });

  it('shows an hour in a tooltip on a key press, with the numbers in words, and lets the arrow keys move', async () => {
    const { root, header } = await mount({ clock: async () => demoClock(4) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    const svg = root.querySelector<SVGElement>('.cdiagram')!;
    expect(root.querySelector('.ctip')).toBeNull();
    svg.dispatchEvent(new FocusEvent('focus'));
    await vi.waitFor(() => expect(root.querySelector('.ctip b')!.textContent).toBe('00:00–01:00'));
    for (let i = 0; i < 21; i++) svg.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.ctip b')!.textContent).toBe('21:00–22:00'));
    const tip = root.querySelector('.ctip')!.textContent!;
    expect(tip).toContain('9 corrections in 66 prompts (13.6 per 100)');
    expect(tip).toContain('thinking 93% of usual (thinking signature length, median of 462)');
    expect(tip).toContain('failed in 726 calls (2.5 per 100)');
    svg.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.ctip b')!.textContent).toBe('00:00–01:00'));
    expect(root.querySelector('.ctip')!.textContent).toContain('10 prompts');
    expect(root.querySelector('.ctip')!.textContent).toContain('too few prompts to read much into');
    svg.dispatchEvent(new FocusEvent('blur'));
    await vi.waitFor(() => expect(root.querySelector('.ctip')).toBeNull());
  });

  it('has a table twin with a row for every hour and every break band', async () => {
    const { root, header } = await mount({ clock: async () => demoClock(4) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    (root.querySelector('.hours .ctoggle') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.hours .ctable')).not.toBeNull());
    expect(root.querySelector('.cdiagram')).toBeNull();
    const tables = root.querySelectorAll('.hours .ctable');
    expect(tables).toHaveLength(2);
    expect(tables[0]!.querySelectorAll('tbody tr')).toHaveLength(24);
    expect(tables[1]!.querySelectorAll('tbody tr')).toHaveLength(4);
    expect(tables[0]!.querySelectorAll('tr.faint').length).toBeGreaterThan(0);
  });

  it('says plainly that there is not enough data, with the counts, and draws no chart', async () => {
    const { root, header } = await mount({ clock: async () => tooFewClock() });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .state')).not.toBeNull());
    expect(hoursText(root)).toContain('Not enough data yet: 120 prompts, 6 corrections.');
    expect(root.querySelector('.cdiagram')).toBeNull();
  });

  it('says plainly that nothing is recorded yet', async () => {
    const { root, header } = await mount({ clock: async () => emptyClock(4) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .state')).not.toBeNull());
    expect(hoursText(root)).toContain('Nothing recorded yet');
    expect(root.querySelector('.cdiagram')).toBeNull();
  });

  it('says it is off when the clock is switched off', async () => {
    const { root, header } = await mount({ clock: async () => ({ ...demoClock(4), enabled: false }) });
    header('HOURS').click();
    await vi.waitFor(() => expect(hoursText(root)).toContain('The steering clock is off'));
  });

  it('is calm when the engine has no clock: not available in this version', async () => {
    const { root, header } = await mount({ clock: async () => { throw new Error('404'); } });
    header('HOURS').click();
    await vi.waitFor(() => expect(hoursText(root)).toContain('Not available in this version'));
    expect(root.querySelector('.cdiagram')).toBeNull();
  });

  it('closes on Esc without closing what is under it', async () => {
    const { root, header } = await mount({ clock: async () => demoClock(4) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours')).not.toBeNull());
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.hours')).toBeNull());
  });
});

describe('the hint beside HOURS', () => {
  it('is a quiet mark with its words, only while the clock has one, and it opens HOURS', async () => {
    const { root } = await mount({ clock: async () => hintClock() });
    const mark = await vi.waitFor(() => {
      const m = root.querySelector<HTMLButtonElement>('header.hd .hourhint');
      expect(m).not.toBeNull();
      return m!;
    });
    expect(mark.textContent).toBe('Your corrections usually rise now');
    expect(mark.title).toBe('Your corrections usually rise now');
    mark.click();
    await vi.waitFor(() => expect(root.querySelector('.hours')).not.toBeNull());
  });

  it('is not there when there is no hint', async () => {
    const { root } = await mount({ clock: async () => demoClock(4) });
    await new Promise((r) => setTimeout(r, 20));
    expect(root.querySelector('.hourhint')).toBeNull();
  });
});

describe('the daily cost chart in USAGE', () => {
  const open = async (over: Partial<Source> = {}) => {
    const usage = vi.fn(async (d: number, s?: 'project' | 'model') => demoUsage(d, s));
    const m = await mount({ usage, ...over });
    m.header('USAGE').click();
    await vi.waitFor(() => expect(m.root.querySelector('.usage .cdiagram')).not.toBeNull());
    return { ...m, usage };
  };

  it('draws a stacked column per day with a legend, split by project to begin with', async () => {
    const { root, usage } = await open();
    expect(usage).toHaveBeenCalledWith(7, 'project');
    expect(root.querySelector('.usage')!.textContent).toContain('DAILY COST, LAST 7 DAYS');
    expect(root.querySelectorAll('.usage .cdiagram .chit')).toHaveLength(7);
    const legend = [...root.querySelectorAll('.usage .clegend li')].map((l) => l.textContent);
    expect(legend).toContain('billing');
    expect(legend.at(-1)).toBe('other');
    // the existing content stays
    for (const t of ['PLAN LIMITS', 'BY AGENT', 'TOP 10 SESSIONS', 'LAST 4 WEEKS']) expect(root.querySelector('.usage')!.textContent).toContain(t);
  });

  it('keeps a colour with its key when the split changes or the window grows', async () => {
    const { root } = await open();
    const fill = (name: string) => {
      const i = [...root.querySelectorAll('.usage .clegend li')].findIndex((l) => l.textContent === name);
      return (root.querySelectorAll<HTMLElement>('.usage .clegend .ckey')[i] as HTMLElement).style.background;
    };
    const billing = fill('billing');
    ([...root.querySelectorAll('.usage .acts button')].find((b) => b.textContent === 'MONTH') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.usage')!.textContent).toContain('LAST 30 DAYS'));
    expect(root.querySelectorAll('.usage .cdiagram .chit')).toHaveLength(30);
    expect(fill('billing')).toBe(billing);
  });

  it('splits by model when asked', async () => {
    const { root, usage } = await open();
    ([...root.querySelectorAll('.usage .acts button')].find((b) => b.textContent === 'BY MODEL') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(usage).toHaveBeenLastCalledWith(7, 'model'));
    await vi.waitFor(() => expect([...root.querySelectorAll('.usage .clegend li')].map((l) => l.textContent)).toContain('claude-opus-5-5'));
  });

  it('shows a column and its total in a tooltip, and a table with the same numbers', async () => {
    const { root } = await open();
    const hit = root.querySelector<SVGElement>('.usage .cdiagram .chit')!;
    hit.dispatchEvent(new FocusEvent('focus'));
    await vi.waitFor(() => expect(root.querySelector('.usage .ctip')).not.toBeNull());
    expect(root.querySelector('.usage .ctip b')!.textContent).toMatch(/^[A-Z][a-z]{2} \d+ [A-Z][a-z]{2} · \$\d/);
    expect(root.querySelectorAll('.usage .ctip p').length).toBeGreaterThan(1);
    (root.querySelector('.usage .ctoggle') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.usage .ctable')).not.toBeNull());
    expect(root.querySelectorAll('.usage .ctable tbody tr')).toHaveLength(7);
  });

  it('shows no chart when the engine has no series yet (an older engine)', async () => {
    const { root } = await mount({ usage: async (d) => ({ ...demoUsage(d), series: undefined }) });
    [...root.querySelectorAll('header.hd button')].find((b) => b.textContent === 'USAGE')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.usage .ubars')).not.toBeNull());
    expect(root.querySelector('.usage .cdiagram')).toBeNull();
  });
});
