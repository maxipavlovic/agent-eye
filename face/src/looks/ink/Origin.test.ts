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

  it('a card says where it was started in its top line, only when that was not a terminal', async () => {
    const store = createStore(fixtureSource(true));
    const root = document.createElement('div');
    document.body.appendChild(root);
    mounted = root;
    render(h(Root, { store }), root);
    await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
    // the top line's facts after the project: the agent (the demo board has two) and the origin
    const facts = [...root.querySelectorAll('.card .meta .mf')].map((c) => c.textContent).filter((f) => f !== 'Claude' && f !== 'Codex');
    expect(facts.length).toBeGreaterThan(0);
    expect(facts.every((c) => ['App', 'IDE', 'Night'].includes(c ?? ''))).toBe(true);
    const total = root.querySelectorAll('.card:not(.idea):not(.row)').length;
    expect(facts.length).toBeLessThan(total);
  });

  it('a night run\'s card has its night mark and does not say Night again in its top line', async () => {
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
    await vi.waitFor(() => expect(root.querySelector('.card .tab.night')).not.toBeNull());
    const card = root.querySelector('.card .tab.night')!.closest('.card')!;
    expect([...card.querySelectorAll('.meta .mf')].map((c) => c.textContent)).not.toContain('Night');
  });
});
