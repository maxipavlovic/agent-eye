// @vitest-environment happy-dom
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Source } from '../../api/types';
import { demoUsage, emptyUsage } from '../../data/usageSample';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { usagePanel } from './Usage';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  globalThis.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as typeof matchMedia;
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});

afterEach(() => {
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
  return root;
}

const usageButton = (root: HTMLElement) => [...root.querySelectorAll('header.hd button')].find((b) => b.textContent === 'USAGE') as HTMLButtonElement;

describe('the USAGE panel', () => {
  it('opens from the header with limits, projects, agents, top sessions and the trend, and says it is an estimate', async () => {
    const usage = vi.fn(async (days: number) => demoUsage(days));
    const root = await mount({ usage });
    expect(root.querySelector('.usage')).toBeNull();
    usageButton(root).click();
    await vi.waitFor(() => expect(root.querySelector('.usage .ubars')).not.toBeNull());
    const text = root.querySelector('.usage')!.textContent!;
    expect(usage).toHaveBeenCalledWith(7);
    expect(text).toContain('API-equivalent estimate, not your bill');
    for (const t of ['PLAN LIMITS', 'THIS WEEK BY PROJECT', 'BY AGENT', 'TOP 10 SESSIONS', 'LAST 4 WEEKS']) expect(text).toContain(t);
    expect(text).toContain('~/code/billing');
    expect(root.querySelectorAll('.usage .ubars')).toHaveLength(4);
    // only the Codex limit (86%) is above 80%, so it alone takes the accent
    const lims = [...root.querySelectorAll('.ulimits li')];
    expect(lims.filter((l) => l.classList.contains('hot'))).toHaveLength(1);
    expect(lims.find((l) => l.classList.contains('hot'))!.textContent).toContain('86%');
    expect(lims.find((l) => l.classList.contains('hot'))!.textContent).toMatch(/seen \d+ min ago/);
  });

  it('asks for a month when MONTH is pressed', async () => {
    const usage = vi.fn(async (days: number) => demoUsage(days));
    const root = await mount({ usage });
    usageButton(root).click();
    await vi.waitFor(() => expect(root.querySelector('.usage .ubars')).not.toBeNull());
    ([...root.querySelectorAll('.usage .acts button')].find((b) => b.textContent === 'MONTH') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(usage).toHaveBeenLastCalledWith(30));
    await vi.waitFor(() => expect(root.querySelector('.usage')!.textContent).toContain('THIS MONTH BY PROJECT'));
  });

  it('shows an empty state and no charts while nothing is counted', async () => {
    const root = await mount({ usage: async (d) => ({ ...emptyUsage(d), backfill_left: 12 }) });
    usageButton(root).click();
    await vi.waitFor(() => expect(root.querySelector('.uempty')).not.toBeNull());
    expect(root.querySelector('.uempty')!.textContent).toContain('12 to go');
    expect(root.querySelector('.usage .ubars')).toBeNull();
  });

  it('says so when the engine cannot report usage, and closes with Esc', async () => {
    const root = await mount({ usage: async () => { throw new Error('404'); } });
    usageButton(root).click();
    await vi.waitFor(() => expect(root.querySelector('.usage')!.textContent).toContain('cannot report usage'));
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await vi.waitFor(() => expect(root.querySelector('.usage')).toBeNull());
  });

  it('opens a top session in the detail panel', async () => {
    const root = await mount({ usage: async (d) => demoUsage(d) });
    usageButton(root).click();
    await vi.waitFor(() => expect(root.querySelector('.usage button.uname')).not.toBeNull());
    (root.querySelector('.usage button.uname') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.usage')).toBeNull());
    expect(usagePanel.value).toBe(false);
  });
});
