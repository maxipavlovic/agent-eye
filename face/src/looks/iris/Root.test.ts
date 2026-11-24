// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Source } from '../../api/types';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { resetFolds } from '../../board/Sections';
import { CODE } from './Header';
import { Root } from './Root';
import { openFold } from '../../board/Sections';

let mounted: HTMLElement | null = null;

async function mount(src: Source = fixtureSource(), ready = (root: HTMLElement) => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5)) {
  const store = createStore(src);
  const root = document.createElement('div');
  document.body.appendChild(root);
  mounted = root;
  render(h(Root, { store }), root);
  await vi.waitFor(() => ready(root));
  const $ = <T extends Element = HTMLElement>(sel: string) => root.querySelector<T>(sel);
  const $$ = <T extends Element = HTMLElement>(sel: string) => [...root.querySelectorAll<T>(sel)];
  return { store, root, $, $$ };
}

beforeAll(() => {
  keepPrefsLocal();
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); resetFolds(); delete document.documentElement.dataset.look; });
afterEach(() => { if (mounted) { render(null, mounted); mounted.remove(); mounted = null; } });

describe('the iris board', () => {
  it('draws the header as cells with the eye, its code and the same controls as every look', async () => {
    const { $, $$ } = await mount();
    expect($('.hd .brand .code')!.textContent).toBe(CODE);
    expect($('.hd .brand svg.eye')).not.toBeNull();
    expect($<HTMLInputElement>('#q')!.placeholder).toBe('Search every session');
    const names = $$('.hd button').map((b) => b.textContent?.trim() || b.getAttribute('title'));
    for (const n of ['Lanes', 'Grid', 'light', 'dark', 'auto', 'USAGE', '+ IDEA']) expect(names).toContain(n);
  });

  it('heads every lane with its count, object, one fact and a timeline with one tick per card', async () => {
    const { $$ } = await mount();
    const lanes = $$('.lanes .col');
    expect(lanes.length).toBeGreaterThanOrEqual(4);
    for (const lane of lanes) {
      const n = lane.querySelectorAll('.cbody [data-id]').length;
      expect(lane.querySelector('.chead .count')!.textContent).toBe(String(n));
      expect(lane.querySelector('.chead svg.lic')).not.toBeNull();
      expect(lane.querySelector('.chead .lfact')!.textContent!.length).toBeGreaterThan(3);
      expect(lane.querySelectorAll('.chead .tline .tk').length).toBe(n);
    }
  });

  it('draws no stamp per session, hangs the marks from the top rule and keeps the title block under the board', async () => {
    const { $, $$, store } = await mount();
    expect($$('.sigil')).toHaveLength(0);
    expect($$('.card .tabs .tab.need').length).toBe(store.needsYou.value);
    const cards = (id: string) => store.board.value!.columns.filter((c) => (c.id === 'done') === (id === 'done')).reduce((s, c) => s + c.cards.length, 0);
    expect($('.tblock')!.textContent).toContain(`${cards('open')} open · ${cards('done')} done · ${store.board.value!.archive_count} archived`);
    expect($('.tblock .code')!.textContent).toBe(CODE);
  });

  it('gives each project a strip: its length is the count, the signal part is what needs you', async () => {
    const { $$ } = await mount();
    const bars = $$('.pbar button');
    expect(bars.length).toBeGreaterThan(2);
    for (const b of bars) {
      const needs = Number(b.querySelector('em')?.textContent ?? 0);
      expect(b.querySelectorAll('.pstrip i.y').length).toBe(needs > 0 ? 1 : 0);
    }
  });

  it('switches to the grid with one-line heads that keep the fact, and keeps that choice as its own', async () => {
    const { $, $$ } = await mount();
    $<HTMLButtonElement>('button[title=Grid]')!.click();
    await vi.waitFor(() => expect($('.gridview')).not.toBeNull());
    expect($$('.gridview .chead.inline small').length).toBeGreaterThan(2);
    expect(localStorage.getItem('eye.iris.layout')).toBe('grid');
    expect(localStorage.getItem('eye.ink.layout')).toBeNull();
  });

  it('keeps the open panel sections under its own name', async () => {
    await mount();
    openFold('conversation');
    expect(localStorage.getItem('eye.iris.panel')).toContain('conversation');
    expect(localStorage.getItem('eye.ink.panel')).toBeNull();
  });

  it('offers the looks in the colour picker when the page has a look, iris pressed', async () => {
    document.documentElement.dataset.look = 'iris';
    const { $, $$ } = await mount();
    $<HTMLButtonElement>('.accbtn')!.click();
    await vi.waitFor(() => expect($('[aria-label=Look]')).not.toBeNull());
    const looks = $$('[aria-label=Look] button');
    expect(looks.map((b) => [b.textContent, b.getAttribute('aria-pressed')])).toEqual([['INK', 'false'], ['IRIS', 'true']]);
    expect($$('.accs .acc')).toHaveLength(8);
  });

  it('shows only the colours where the page has one look (the offline sample)', async () => {
    const { $ } = await mount();
    $<HTMLButtonElement>('.accbtn')!.click();
    await vi.waitFor(() => expect($('.accs')).not.toBeNull());
    expect($('[aria-label=Look]')).toBeNull();
  });

  it('welcomes a new user with the eye instead of empty lanes', async () => {
    const src = fixtureSource();
    const empty: Source = {
      ...src,
      board: async () => {
        const b = await src.board();
        return { ...b, columns: b.columns.map((c) => ({ ...c, cards: [] })), archive_count: 0 };
      },
    };
    const { $ } = await mount(empty, (root) => expect(root.querySelector('.welcome')).not.toBeNull());
    expect($('.welcome .weye svg')).not.toBeNull();
    expect($('.welcome h2')!.textContent).toBe('NO SESSIONS YET');
    expect($('.tblock')).toBeNull();
  });
});
