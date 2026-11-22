// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { originChip } from './pieces';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); });
afterEach(() => {
  if (mounted) { render(null, mounted); mounted.remove(); mounted = null; }
  document.body.innerHTML = '';
});

describe('where a session was started', () => {
  it('names each origin but the terminal', () => {
    expect(originChip('app')).toBe('App');
    expect(originChip('headless')).toBe('Headless');
    expect(originChip('desktop')).toBe('Desktop');
    expect(originChip('ide')).toBe('IDE');
    expect(originChip('web')).toBe('Web');
    expect(originChip('night')).toBe('Night');
    expect(originChip('terminal')).toBeNull();
    expect(originChip(null)).toBeNull();
    expect(originChip(undefined)).toBeNull();
  });

  it('a card shows the chip only when the session did not start in a terminal', async () => {
    const store = createStore(fixtureSource(true));
    const root = document.createElement('div');
    document.body.appendChild(root);
    mounted = root;
    render(h(Root, { store }), root);
    await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
    const chips = [...root.querySelectorAll('.card .mk.origin')].map((c) => c.textContent);
    expect(chips.length).toBeGreaterThan(0);
    expect(chips.every((c) => ['App', 'IDE', 'Night'].includes(c ?? ''))).toBe(true);
    const total = root.querySelectorAll('.card').length;
    expect(chips.length).toBeLessThan(total);
  });

  it('a night run\'s card has its night mark and no second Night chip', async () => {
    const src = fixtureSource(true);
    const board = src.board;
    src.board = async () => {
      const b = await board();
      const c = b.columns.flatMap((col) => col.cards).find((x) => x.night)!;
      c.origin = 'night';
      return b;
    };
    const store = createStore(src);
    const root = document.createElement('div');
    document.body.appendChild(root);
    mounted = root;
    render(h(Root, { store }), root);
    await vi.waitFor(() => expect(root.querySelector('.card .mk.night')).not.toBeNull());
    const card = root.querySelector('.card .mk.night')!.closest('.card')!;
    expect(card.querySelector('.mk.origin')).toBeNull();
  });
});
