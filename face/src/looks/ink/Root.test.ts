// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Agent, Source } from '../../api/types';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { ideaForm } from './IdeaForm';
import { Root } from './Root';

const CLAUDE: Agent = { id: 'claude', name: 'Claude Code', short: 'Claude', default: true };
const CODEX: Agent = { id: 'codex', name: 'Codex CLI', short: 'Codex', default: false };

let mounted: HTMLElement | null = null;

/** The sample board with nothing on it: a user who has not started a session yet. */
function emptySource(agents: Agent[] = [CLAUDE], archived = 0): Source {
  const src = fixtureSource();
  return {
    ...src,
    agents: async () => ({ agents }),
    board: async () => {
      const b = await src.board();
      return { ...b, columns: b.columns.map((c) => ({ ...c, cards: [] })), archive_count: archived };
    },
  };
}

const sampleCards = (root: HTMLElement) => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5);

/** Draws the board over the sample data and waits until `ready` holds (default: the cards are there). */
async function mount(src: Source = fixtureSource(), ready: (root: HTMLElement) => void = sampleCards) {
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

const key = (k: string) => document.body.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
const card = (root: HTMLElement, lane: string, title: string) =>
  [...root.querySelectorAll<HTMLElement>(`.c-${lane} .card`)].find((c) => c.textContent?.includes(title))!;

beforeAll(() => {
  keepPrefsLocal();
  // happy-dom has no layout; these only need to exist.
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); });
afterEach(() => {
  if (mounted) { render(null, mounted); mounted.remove(); mounted = null; }
  ideaForm.value = null;
  document.body.innerHTML = '';
});

describe('the board', () => {
  it('draws every lane, counts what needs you, and blinks the stopped session', async () => {
    const { $, $$ } = await mount();
    expect($('.c-needs_you .count')!.textContent).toBe('5');
    for (const lane of ['ideas', 'running', 'waiting', 'paused']) expect($(`.c-${lane}`), lane).not.toBeNull();
    expect($('.rail.folded.c-done')).not.toBeNull();
    const blinking = $('.chip.now')!;
    expect(blinking.textContent).toBe('YOU');
    expect(blinking.getAttribute('title')).toBe('Claude is stopped until you answer in the session');
    expect($$('.c-waiting .chip.others .lbl').map((e) => e.textContent)).toEqual(expect.arrayContaining(['PRIYA', 'DANA']));
    await vi.waitFor(() => expect(document.title).toBe('agent-eye - 5 need you'));
  });

  it('switches to the grid and remembers it', async () => {
    const { $ } = await mount();
    $('.layout button:nth-child(2)')!.click();
    await vi.waitFor(() => expect($('.board.gridview')).not.toBeNull());
    expect(localStorage.getItem('eye.ink.layout')).toBe('grid');
    expect($('.needsbox .ngrid .card.need')).not.toBeNull();
    $('.layout button:nth-child(1)')!.click();
    await vi.waitFor(() => expect($('.board.lanes')).not.toBeNull());
  });

  it('folds a lane to a rail and opens it again', async () => {
    const { $ } = await mount();
    $('.c-ideas .foldhit')!.click();
    await vi.waitFor(() => expect($('.rail.folded.c-ideas')).not.toBeNull());
    expect(JSON.parse(localStorage.getItem('eye.ink.fold')!)).toContain('ideas');
    $('.rail.folded.c-ideas')!.click();
    await vi.waitFor(() => expect($('section.col.c-ideas')).not.toBeNull());
  });

  it('filters by project', async () => {
    const { $, $$ } = await mount();
    const all = $$('.card').length;
    $$('.pbar button')[1]!.click();
    await vi.waitFor(() => expect($$('.card').length).toBeLessThan(all));
    expect($$('.card .ptag')).toHaveLength(0);
    $$('.pbar button')[0]!.click();
    await vi.waitFor(() => expect($$('.card').length).toBe(all));
  });
});

describe('a board with nothing on it', () => {
  const welcomed = (root: HTMLElement) => expect(root.querySelector('.welcome')).not.toBeNull();

  it('points a new user to their agent instead of showing empty lanes', async () => {
    const { $, $$ } = await mount(emptySource(), (root) => expect(root.querySelector('.welcome p')!.textContent).toContain('Claude Code'));
    expect($('.welcome svg')).not.toBeNull();
    expect($('.welcome h2')!.textContent).toBe('NO SESSIONS YET');
    expect($$('.welcome p').map((p) => p.textContent)).toEqual([
      'Start a session in Claude Code and it shows up here.',
      'Prefer the terminal? Run eye there.',
    ]);
    expect($('.welcome code')!.textContent).toBe('eye');
    expect($('.board')).toBeNull();
    expect($('.pbar')).toBeNull();
    expect($('.hd')).not.toBeNull();
  });

  it('names every enabled agent', async () => {
    const { $ } = await mount(emptySource([CLAUDE, CODEX]), (root) => expect(root.querySelector('.welcome p')!.textContent).toContain('Codex CLI'));
    expect($('.welcome p')!.textContent).toBe('Start a session in Claude Code or Codex CLI and it shows up here.');
  });

  it('says "your coding agent" while no agent is known', async () => {
    const { $ } = await mount(emptySource([]), welcomed);
    expect($('.welcome p')!.textContent).toBe('Start a session in your coding agent and it shows up here.');
  });

  it('is not shown when sessions are only in the archive', async () => {
    const { $ } = await mount(emptySource([CLAUDE], 3), (root) => expect(root.querySelector('.board')).not.toBeNull());
    expect($('.welcome')).toBeNull();
  });

  it('is not shown when an idea is parked', async () => {
    const src = emptySource();
    const idea = (await fixtureSource().board()).columns.find((c) => c.id === 'ideas')!.cards[0]!;
    const board = await src.board();
    board.columns.find((c) => c.id === 'ideas')!.cards.push(idea);
    const { $ } = await mount({ ...src, board: async () => structuredClone(board) }, (root) => expect(root.querySelector('.card')).not.toBeNull());
    expect($('.welcome')).toBeNull();
  });

  it('gives way to the board once a session shows up', async () => {
    const sample = fixtureSource();
    const src = emptySource();
    let live = false;
    let change = () => {};
    const { $ } = await mount(
      { ...src, board: () => (live ? sample.board() : src.board()), subscribe: (onChange) => { change = onChange; return () => {}; } },
      welcomed,
    );
    live = true;
    change();
    await vi.waitFor(() => expect($('.board.lanes')).not.toBeNull());
    expect($('.welcome')).toBeNull();
  });
});

/** The sample board with every third session run by Codex, and both agents enabled. */
function mixedSource(): Source {
  const src = fixtureSource();
  const codex = new Set<string>();
  const asCodex = <T extends { id: string; agent: string; agent_name: string }>(c: T): T => ({ ...c, agent: 'codex', agent_name: 'Codex' });
  return {
    ...src,
    agents: async () => ({ agents: [CLAUDE, CODEX] }),
    board: async () => {
      const b = await src.board();
      let n = 0;
      return {
        ...b,
        columns: b.columns.map((col) => ({
          ...col,
          cards: col.cards.map((c) => {
            if (c.kind === 'idea' || n++ % 3 !== 0) return c;
            codex.add(c.id);
            return asCodex(c);
          }),
        })),
      };
    },
    card: async (id) => {
      const d = await src.card(id);
      return codex.has(d.id) ? asCodex(d) : d;
    },
  };
}

describe('a board of several agents', () => {
  it('labels each card with its agent, and only when there is more than one', async () => {
    const { $$ } = await mount(mixedSource(), (root) => expect(root.querySelectorAll('.card .agent-tag').length).toBeGreaterThan(3));
    const tags = $$('.card .agent-tag').map((t) => t.textContent);
    expect(new Set(tags)).toEqual(new Set(['Claude', 'Codex']));
    // one label in the meta line of every full card; rows and ideas have no meta line
    const metas = $$('.card .meta');
    expect(metas.length).toBeGreaterThan(3);
    for (const m of metas) expect(m.querySelectorAll('.agent-tag')).toHaveLength(1);
    expect($$('.card.row .agent-tag, .card.idea .agent-tag')).toHaveLength(0);
    expect($$('.card .meta .agent-tag').every((t) => t.parentElement!.classList.contains('meta'))).toBe(true);
  });

  it('names the agent in the header of the panel', async () => {
    const { $, $$ } = await mount(mixedSource(), (root) => expect(root.querySelectorAll('.card .agent-tag').length).toBeGreaterThan(3));
    $$('.card').find((c) => c.querySelector('.agent-tag')?.textContent === 'Codex')!.click();
    await vi.waitFor(() => expect($('.page .dh .agent-tag')?.textContent).toBe('Codex'));
  });

  it('adds nothing with one agent: the sample board keeps its look', async () => {
    const { $$, root } = await mount();
    expect($$('.agent-tag')).toHaveLength(0);
    card(root, 'needs_you', 'date library').click();
    await vi.waitFor(() => expect(root.querySelector('.page h2')).not.toBeNull());
    expect($$('.page .agent-tag')).toHaveLength(0);
  });
});

describe('the panel', () => {
  it('opens a session with its parts, and Esc closes it', async () => {
    const { root, $, $$, store } = await mount();
    card(root, 'needs_you', 'date library').click();
    await vi.waitFor(() => expect($('.page h2')?.textContent).toBe('Upgrade the date library'));
    expect($('.qmove button[aria-pressed="true"]')!.textContent).toBe('NEEDS YOU');
    expect($('.page .chip.now')).not.toBeNull();
    expect($$('.page .sec h4').map((h4) => h4.textContent)).toEqual(expect.arrayContaining(['TOPICS3', 'OUTPUTS2']));
    expect($('.receipt code')!.textContent).toContain('claude --resume');
    key('Escape');
    await vi.waitFor(() => expect(store.selected.value).toBeNull());
    expect($('.page')).toBeNull();
  });

  it('names the agent in the owner chip of a session it works on', async () => {
    const { root, $ } = await mount();
    card(root, 'running', 'dark mode').click();
    await vi.waitFor(() => expect($('.page .chip.agent')).not.toBeNull());
    expect($('.page .chip.agent .lbl')!.textContent).toBe('CLAUDE');
  });

  it('walks the cards with the arrow keys', async () => {
    const { root, $, store } = await mount();
    card(root, 'needs_you', 'date library').click();
    await vi.waitFor(() => expect($('.page h2')).not.toBeNull());
    const first = store.selected.value;
    key('ArrowRight');
    await vi.waitFor(() => expect(store.selected.value).not.toBe(first));
    key('ArrowLeft');
    await vi.waitFor(() => expect(store.selected.value).toBe(first));
  });

  it('moves the open session with a key and offers Undo for Done', async () => {
    const { root, $, $$, store } = await mount();
    card(root, 'needs_you', 'flaky login').click();
    await vi.waitFor(() => expect($('.page h2')?.textContent).toBe('Fix the flaky login test'));
    key('d');
    await vi.waitFor(() => expect(store.board.value!.columns.find((c) => c.id === 'done')!.cards.some((c) => c.title.includes('flaky login'))).toBe(true));
    await vi.waitFor(() => expect($$('.toast').some((t) => t.textContent?.includes('Moved to Done'))).toBe(true));
    $('.toast .tact')!.click();
    await vi.waitFor(() => expect(store.board.value!.columns.find((c) => c.id === 'needs_you')!.cards.some((c) => c.title.includes('flaky login'))).toBe(true));
  });

  it('loads the conversation when its section is opened', async () => {
    const { root, $, $$ } = await mount();
    card(root, 'needs_you', 'date library').click();
    await vi.waitFor(() => expect($('.page h2')).not.toBeNull());
    $$('.fsec .fhead').find((b) => b.textContent?.startsWith('CONVERSATION'))!.click();
    await vi.waitFor(() => expect($$('.conv .msg').length).toBeGreaterThan(1));
    expect($$('.conv .msg.agent .who b')[0]!.textContent).toBe('Claude');
    expect($$('.conv .msg.you .who b')[0]!.textContent).toBe('You');
  });
});

describe('search', () => {
  it('marks the words in the results and in the panel opened from one', async () => {
    const { root, $, $$, store } = await mount();
    store.search('retry');
    await vi.waitFor(() => expect($$('.results .card.hit').length).toBeGreaterThan(2));
    expect($$('.results mark').length).toBeGreaterThan(0);
    $$('.results .card.hit').find((c) => c.textContent?.includes('webhooks'))!.click();
    await vi.waitFor(() => expect($('.page h2 mark')?.textContent?.toLowerCase()).toBe('retry'));
    expect($('.page .match')).not.toBeNull();
    key('Escape');
    key('Escape');
    await vi.waitFor(() => expect(root.querySelector('.results')).toBeNull());
  });

  it('/ and ⌘K / Ctrl-K go back to the search box with its text selected', async () => {
    const { $, store } = await mount();
    const q = $<HTMLInputElement>('#q')!;
    store.search('retry');
    await vi.waitFor(() => expect(q.value).toBe('retry'));
    for (const press of [{ key: '/' }, { key: 'k', metaKey: true }, { key: 'K', ctrlKey: true }]) {
      q.blur();
      document.body.dispatchEvent(new KeyboardEvent('keydown', { ...press, bubbles: true }));
      expect(document.activeElement).toBe(q);
      expect([q.selectionStart, q.selectionEnd]).toEqual([0, 5]);
    }
  });
});

describe('the card menu', () => {
  it('opens on right-click and marks the session done', async () => {
    const { root, $, $$, store } = await mount();
    card(root, 'needs_you', 'flaky login').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 200, clientY: 200 }));
    await vi.waitFor(() => expect($('.ctx')).not.toBeNull());
    expect($('.ctx .ct')!.textContent).toBe('Fix the flaky login test');
    $$('.ctx button').find((b) => b.textContent === 'MARK DONE')!.click();
    await vi.waitFor(() => expect(store.board.value!.columns.find((c) => c.id === 'done')!.cards.some((c) => c.title.includes('flaky login'))).toBe(true));
    expect($('.ctx')).toBeNull();
  });
});

describe('the idea form', () => {
  it('names the agent in its labels and shows no picker for one agent', async () => {
    const { $ } = await mount();
    $('.btn.plus')!.click();
    await vi.waitFor(() => expect($('form.modal')).not.toBeNull());
    expect($('form.modal')!.textContent).toContain('PROMPT FOR CLAUDE');
    expect($('form.modal .hint')!.textContent).toBe('For you. Never sent to Claude.');
    expect($('.modal .pick')).toBeNull();
  });

  it('offers a picker for several agents and sends the chosen one', async () => {
    const addIdea = vi.fn(fixtureSource().addIdea);
    const src: Source = { ...fixtureSource(), agents: async () => ({ agents: [CLAUDE, CODEX] }), addIdea };
    const { $, $$ } = await mount(src);
    $('.btn.plus')!.click();
    await vi.waitFor(() => expect($$('.modal .pick button')).toHaveLength(2));
    expect($$('.modal .pick button').map((b) => b.getAttribute('aria-pressed'))).toEqual(['true', 'false']);
    $$('.modal .pick button')[1]!.click();
    await vi.waitFor(() => expect($('form.modal')!.textContent).toContain('PROMPT FOR CODEX'));
    $<HTMLInputElement>('.modal input[name="title"]')!.value = 'Compare agents';
    $('form.modal')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(addIdea).toHaveBeenCalled());
    expect(addIdea.mock.calls[0]![0]).toMatchObject({ title: 'Compare agents', agent: 'codex' });
  });

  it('edits an idea from its panel', async () => {
    const { root, $ } = await mount();
    card(root, 'ideas', 'Retry budget').click();
    await vi.waitFor(() => expect($('.page .acts .btn')).not.toBeNull());
    const edit = [...root.querySelectorAll<HTMLElement>('.page .acts .btn')].find((b) => b.textContent === 'EDIT')!;
    edit.click();
    await vi.waitFor(() => expect($<HTMLInputElement>('.modal input[name="title"]')?.value).toBe('Retry budget'));
    expect($('.modal h2')!.textContent).toContain('EDIT IDEA');
  });
});
