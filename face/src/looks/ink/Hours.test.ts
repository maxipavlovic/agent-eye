// @vitest-environment happy-dom
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Source } from '../../api/types';
import { demoClock, emptyClock } from '../../data/clockSample';
import { hintClock, tooFewClock } from '../../data/clockStates';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { hoursPanel } from '../../board/Hours';
import { ApiError } from '../../data/live';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  globalThis.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as typeof matchMedia;
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});

afterEach(() => {
  hoursPanel.value = false;
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
  it('opens from the button, says the verdict and draws the three strips and the break chart', async () => {
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
    // only the by-agent table: the hour and break tables are the table view
    expect(root.querySelectorAll('.hours .ctable')).toHaveLength(1);
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
    const svg = root.querySelector<HTMLElement>('.cfocus')!;
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
    expect(tables).toHaveLength(3);
    expect(tables[0]!.querySelectorAll('tbody tr')).toHaveLength(24);
    expect(tables[1]!.querySelectorAll('tbody tr')).toHaveLength(4);
    expect(tables[0]!.querySelectorAll('tr.faint').length).toBeGreaterThan(0);
  });

  it('draws the usual line as a tick on every hour with thinking, and cuts a long swing at the edge with a mark', async () => {
    const c = demoClock(4);
    const hours = c.hours.map((x) => (x.h === 3 ? { ...x, think: 5_000, think_n: 100 } : x.h === 4 ? { ...x, think: 10, think_n: 100 } : x));
    const { root, header } = await mount({ clock: async () => ({ ...c, hours }) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    const svg = root.querySelector('.cdiagram')!;
    expect(svg.querySelectorAll('.ctick').length).toBe(hours.filter((x) => x.think !== null).length);
    expect(svg.querySelectorAll('.ccut')).toHaveLength(2);
    expect(svg.textContent).toContain('≥150%');
    expect(svg.textContent).toContain('≤50%');
  });

  it('names the bad stretch with the engine\'s part of the day, wherever it falls', async () => {
    const c = { ...demoClock(4), bad: [{ from_h: 22, to_h: 4, label: 'night' as const }] };
    const { root, header } = await mount({ clock: async () => c });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    const labels = [...root.querySelectorAll('.cband-label')];
    expect(labels.map((l) => l.textContent)).toEqual(['your night']);
    // a band that starts near midnight keeps its label inside the picture, ending where the first piece ends
    expect(labels[0]!.getAttribute('text-anchor')).toBe('end');
  });

  it('lists each agent, with a dash where it has no such signal', async () => {
    const { root, header } = await mount({ clock: async () => demoClock(4) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    const rows = [...root.querySelectorAll('.hours .ctable tbody tr')].map((r) => [...r.children].map((c) => c.textContent));
    expect(rows).toHaveLength(2);
    expect(rows[0]![3]).toBe('−7%');
    expect(rows[1]![0]).not.toBe('');
    expect(rows[1]!.slice(3)).toEqual(['—', '—']);
  });

  it('keeps the chart on screen while another number of weeks loads', async () => {
    let release: (c: ReturnType<typeof demoClock>) => void = () => {};
    const clock = vi.fn((w: number) => (w === 8 ? new Promise<ReturnType<typeof demoClock>>((r) => { release = r; }) : Promise.resolve(demoClock(w))));
    const { root, header } = await mount({ clock });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    ([...root.querySelectorAll('.hours .acts button')].find((b) => b.textContent === '8 WEEKS') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(clock).toHaveBeenLastCalledWith(8));
    expect(root.querySelector('.hours .cdiagram')).not.toBeNull();
    expect(hoursText(root)).not.toContain('Loading');
    release(demoClock(8));
  });

  it('says why corrections are zero when they are not judged', async () => {
    const off = { ...tooFewClock(), judging: 'off' as const, judging_reason: '[clock] llm is off' };
    const { root, header } = await mount({ clock: async () => off });
    header('HOURS').click();
    await vi.waitFor(() => expect(hoursText(root)).toContain('Corrections are not being judged: [clock] llm is off.'));
  });

  it('says it above the chart, and no longer counts prompts as waiting for the nightly pass, when old corrections remain', async () => {
    const { root, header } = await mount({ clock: async () => ({ ...demoClock(4), judging: 'off' as const, judging_reason: '[clock] llm is off' }) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    expect(hoursText(root)).toContain('Corrections are not being judged: [clock] llm is off.');
    expect(hoursText(root)).not.toContain('not judged yet');
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
    const { root, header } = await mount({ clock: async () => { throw new ApiError('not found', 404); } });
    header('HOURS').click();
    await vi.waitFor(() => expect(hoursText(root)).toContain('Not available in this version'));
    expect(root.querySelector('.cdiagram')).toBeNull();
  });

  it('names the agent whose calls the tool strip shows', async () => {
    const { root, header } = await mount({ clock: async () => demoClock(4) });
    header('HOURS').click();
    await vi.waitFor(() => expect(root.querySelector('.hours .cdiagram')).not.toBeNull());
    expect(root.querySelector('.hours .cdiagram')!.textContent).toMatch(/TOOL ERRORS · \S+'s failed calls per 100/);
  });

  it('says what went wrong when the clock fails for any other reason', async () => {
    const { root, header } = await mount({ clock: async () => { throw new ApiError('database is locked', 500); } });
    header('HOURS').click();
    await vi.waitFor(() => expect(hoursText(root)).toContain('Could not load the hours: database is locked'));
    expect(hoursText(root)).not.toContain('Not available in this version');
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
