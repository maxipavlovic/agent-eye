// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Source } from '../../api/types';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { ignoreText, sendText } from './Loose';

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

/** The demo board, with the loose-end calls recorded. */
async function mount() {
  const base = fixtureSource(true);
  const acts: Array<[string, string]> = [];
  const sends: Array<[string, boolean]> = [];
  const src: Source = {
    ...base,
    looseAct: (a, k) => { acts.push([a, k]); return base.looseAct(a, k); },
    looseSend: (s, d) => { sends.push([s, d]); return base.looseSend(s, d); },
  };
  const store = createStore(src);
  const root = document.createElement('div');
  document.body.appendChild(root);
  mounted = root;
  render(h(Root, { store }), root);
  await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
  const $ = <T extends Element = HTMLElement>(sel: string) => root.querySelector<T>(sel);
  const $$ = <T extends Element = HTMLElement>(sel: string) => [...root.querySelectorAll<T>(sel)];
  return { store, root, acts, sends, $, $$ };
}

const text = (e: Element | null) => e?.textContent ?? '';
const button = (root: ParentNode, label: string) => [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).trim() === label);

describe('loose ends on the board', () => {
  it('a card says how many things its session left, in a calm chip', async () => {
    const { $$ } = await mount();
    const chips = $$('.card .mk.loose').map((c) => text(c));
    expect(chips).toEqual(expect.arrayContaining(['3 loose ends', '1 loose end']));
    // neutral ink: not the accent chip family of Needs you
    expect($$('.card .mk.loose').every((c) => !c.classList.contains('chip'))).toBe(true);
  });

  it('no chip on a card that left nothing, and none when the count is 0', async () => {
    const { $$ } = await mount();
    const withChip = $$('.card').filter((c) => c.querySelector('.mk.loose')).length;
    expect(withChip).toBe(2);
  });
});

describe('the LEFT BEHIND section', () => {
  async function openCard(m: Awaited<ReturnType<typeof mount>>, chip: string) {
    const card = m.$$('.card').find((c) => text(c.querySelector('.mk.loose')) === chip)!;
    card.click();
    await vi.waitFor(() => expect(m.$('.page .dloose')).not.toBeNull());
  }

  it('lists each item with the buttons for its kind', async () => {
    const m = await mount();
    await openCard(m, '3 loose ends');
    const rows = m.$$('.dloose li');
    expect(rows.map((r) => text(r.querySelector('.ll')))).toEqual([
      'node vite (pid 38090) is still serving port 4321, started 6d ago',
      'node api (pid 38112) is still serving port 4350, started 6d ago',
      '3 commits in web are on no remote',
    ]);
    const labels = (r: Element) => [...r.querySelectorAll('button')].map((b) => text(b));
    expect(labels(rows[0]!)).toEqual(['STOP', 'IGNORE']);
    expect(labels(rows[2]!)).toEqual(['COPY COMMAND', 'IGNORE']);
    expect(button(m.root, 'SEND BACK TO THE AGENT')).toBeDefined();
  });

  it('asks first, in the page, with the exact words, and a cancel does nothing', async () => {
    const m = await mount();
    await openCard(m, '3 loose ends');
    button(m.$$('.dloose li')[0]!, 'STOP')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).not.toBeNull());
    expect(text(m.$('.lask p'))).toBe('Stop node vite (pid 38090, port 4321)?');
    expect(m.acts).toEqual([]);
    button(m.root, 'CANCEL')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).toBeNull());
    expect(m.acts).toEqual([]);
  });

  it('a yes calls the stop route for that key, and the item leaves the list and the chip', async () => {
    const m = await mount();
    await openCard(m, '3 loose ends');
    button(m.$$('.dloose li')[0]!, 'STOP')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).not.toBeNull());
    button(m.root, 'YES')!.click();
    await vi.waitFor(() => expect(m.$$('.dloose li').length).toBe(2));
    expect(m.acts).toEqual([['stop', 'proc:38090:1']]);
    await vi.waitFor(() => expect(m.$$('.card .mk.loose').map((c) => text(c))).toContain('2 loose ends'));
  });

  it('ignore asks, then calls the ignore route; REMOVE sends the worktree key', async () => {
    const m = await mount();
    await openCard(m, '3 loose ends');
    button(m.$$('.dloose li')[2]!, 'IGNORE')!.click();
    await vi.waitFor(() => expect(text(m.$('.lask p'))).toBe('Stop listing 3 commits in web?'));
    button(m.root, 'YES')!.click();
    await vi.waitFor(() => expect(m.acts).toEqual([['ignore', 'unpushed:/Users/you/code/web:abc1234']]));
    m.store.close();
    await vi.waitFor(() => expect(m.$('.page')).toBeNull());
    await openCard(m, '1 loose end');
    button(m.$$('.dloose li')[0]!, 'REMOVE')!.click();
    await vi.waitFor(() => expect(text(m.$('.lask p'))).toBe('Remove worktree /Users/you/code/web-retry?'));
    button(m.root, 'YES')!.click();
    await vi.waitFor(() => expect(m.acts.at(-1)).toEqual(['remove', 'worktree:/Users/you/code/web-retry']));
  });

  it('copying the command asks nothing and never calls an acting route', async () => {
    const m = await mount();
    await openCard(m, '3 loose ends');
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    button(m.$$('.dloose li')[2]!, 'COPY COMMAND')!.click();
    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith("cd '/Users/you/code/web' && git push --all"));
    expect(m.$('.lask')).toBeNull();
    expect(m.acts).toEqual([]);
  });

  it('SEND BACK asks, then opens the session with the message', async () => {
    const m = await mount();
    await openCard(m, '3 loose ends');
    button(m.root, 'SEND BACK TO THE AGENT')!.click();
    await vi.waitFor(() => expect(text(m.$('.lask p'))).toBe(sendText('Claude', 3)));
    expect(m.sends).toEqual([]);
    button(m.root, 'YES')!.click();
    await vi.waitFor(() => expect(m.sends.length).toBe(1));
    expect(m.sends[0]![1]).toBe(false);
  });

  it('writes the questions the way the CLI and the picker do', () => {
    expect(ignoreText({ what: 'x (pid 1)' } as never)).toBe('Stop listing x (pid 1)?');
    expect(sendText('Codex', 1)).toBe('Open Codex in this session with a message that lists the thing it left behind?');
  });
});
