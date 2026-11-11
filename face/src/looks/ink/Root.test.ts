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
    expect($('.c-needs_you .count')!.textContent).toBe('6');
    for (const lane of ['ideas', 'running', 'waiting', 'paused']) expect($(`.c-${lane}`), lane).not.toBeNull();
    expect($('.rail.folded.c-done')).not.toBeNull();
    const blinking = $('.chip.now')!;
    expect(blinking.textContent).toBe('YOU');
    expect(blinking.getAttribute('title')).toBe('Claude is stopped until you answer in the session');
    expect($$('.c-waiting .chip.others .lbl').map((e) => e.textContent)).toEqual(expect.arrayContaining(['PRIYA', 'SAM']));
    await vi.waitFor(() => expect(document.title).toBe('agent-eye - 6 need you'));
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

describe('what a session does on its own', () => {
  it('shows a background task, a loop and the remote control mark on the cards in Running', async () => {
    const { $$ } = await mount();
    const lines = $$('.c-running .card .activity').map((e) => e.textContent);
    expect(lines).toEqual(['background: npm run dev · 4 min', '↻ every 10 min · next 14:20']);
    expect($$('.c-running .card .rc')).toHaveLength(1);
    expect($$('.c-needs_you .card .rc, .c-needs_you .card .activity')).toHaveLength(0);
  });

  it('lists them in the panel, with the link to the remote session', async () => {
    const { $, $$, store } = await mount();
    const running = (await fixtureSource().board()).columns.find((c) => c.id === 'running')!.cards;
    store.open(running.find((c) => c.remote_url)!.id);
    await vi.waitFor(() => expect($('.remote a')).not.toBeNull());
    const a = $<HTMLAnchorElement>('.remote a')!;
    expect(a.getAttribute('rel')).toBe('noopener');
    expect(a.getAttribute('href')).toMatch(/^https:\/\/claude\.ai\/code\//);
    expect($('.remote')!.textContent).toBe('REMOTE CONTROL on · open in browser');
    store.open(running.find((c) => c.background?.length)!.id);
    await vi.waitFor(() => expect($$('.druns li').map((e) => e.textContent)).toEqual(['SHELLnpm run dev · 4 min']));
    expect($('.remote')).toBeNull();
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

describe('reminders', () => {
  it('puts a due card first in Needs you with its reason, and a chip on one that is coming', async () => {
    const { $, $$ } = await mount();
    const first = $$('.c-needs_you .card')[0]!;
    expect(first.querySelector('.remind.due')!.textContent).toBe("⏰ check on: Dana's CDN rule");
    expect(first.textContent).toContain('Cache the product list');
    expect($$('.c-waiting .rchip').map((e) => e.textContent)).toEqual([expect.stringMatching(/^⏰ (tomorrow|today \d\d:\d\d)$/)]);
    expect($$('.c-needs_you .rchip')).toHaveLength(0);
  });

  it('sets a reminder from the menu, and clearing is offered once one is set', async () => {
    const { root, $, $$, store } = await mount();
    const open = (title: string) => {
      card(root, 'paused', title).dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 200, clientY: 200 }));
      return vi.waitFor(() => expect($('.ctx')).not.toBeNull());
    };
    await open('Changelog generator');
    expect($$('.ctx button').map((b) => b.textContent)).not.toContain('CLEAR REMINDER');
    $$('.ctx button').find((b) => b.textContent === 'TOMORROW 9:00')!.click();
    await vi.waitFor(() => expect(store.toasts.value.some((t) => t.text.startsWith('Reminder set: tomorrow 09:00'))).toBe(true));
    await vi.waitFor(() => expect(card(root, 'paused', 'Changelog generator').querySelector('.rchip')!.textContent).toBe('⏰ tomorrow'));
    await open('Changelog generator');
    $$('.ctx button').find((b) => b.textContent === 'CLEAR REMINDER')!.click();
    await vi.waitFor(() => expect(card(root, 'paused', 'Changelog generator').querySelector('.rchip')).toBeNull());
  });

  it('opens the form for a date and sends it as words', async () => {
    const remind = vi.fn(fixtureSource().remind);
    const { root, $, $$ } = await mount({ ...fixtureSource(), remind });
    card(root, 'paused', 'Changelog generator').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 200, clientY: 200 }));
    await vi.waitFor(() => expect($('.ctx')).not.toBeNull());
    $$('.ctx button').find((b) => b.textContent === 'PICK A DATE…')!.click();
    await vi.waitFor(() => expect($('form.modal')).not.toBeNull());
    const f = $<HTMLFormElement>('form.modal')!;
    const field = (n: string) => f.elements.namedItem(n) as HTMLInputElement;
    field('date').value = '2099-03-04';
    field('time').value = '';
    field('note').value = 'before the freeze';
    f.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(remind).toHaveBeenCalled());
    expect(remind.mock.calls[0]![1]).toEqual({ when: '2099-03-04 09:00', note: 'before the freeze' });
    await vi.waitFor(() => expect($('form.modal')).toBeNull());
  });

  it('shows the reminder in the panel with a way to change or clear it', async () => {
    const { root, $, $$ } = await mount();
    card(root, 'needs_you', 'Cache the product list').click();
    await vi.waitFor(() => expect($('.page')).not.toBeNull());
    await vi.waitFor(() => expect($$('.page .sec h4').some((h) => h.textContent === 'REMINDER')).toBe(true));
    expect($('.page .remind')!.textContent).toBe("⏰ check on: Dana's CDN rule");
    expect($$('.page .acts .btn').map((b) => b.textContent)).toEqual(expect.arrayContaining(['TOMORROW 9:00', 'PICK A DATE…', 'CLEAR']));
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

describe('an idea whose folder does not exist', () => {
  const buttons = (root: HTMLElement) => [...root.querySelectorAll<HTMLButtonElement>('.page .acts .btn')];

  it('is marked on its card and offers to create the folder and start', async () => {
    const startIdea = vi.fn(fixtureSource().startIdea);
    const { root, $, $$ } = await mount({ ...fixtureSource(), startIdea });
    expect(card(root, 'ideas', 'Migration guide').querySelector('.fmark')).not.toBeNull();
    expect(card(root, 'ideas', 'Retry budget').querySelector('.fmark')).toBeNull();
    expect(card(root, 'ideas', 'Retry budget').querySelector('.btn')!.textContent).toBe('START');
    card(root, 'ideas', 'Migration guide').click();
    await vi.waitFor(() => expect($$('.page .acts .btn').length).toBeGreaterThan(0));
    const start = buttons(root)[0]!;
    expect(start.textContent).toBe('CREATE FOLDER + START');
    expect($('.page .fnote')!.textContent).toContain('Start will create it');
    start.click();
    await vi.waitFor(() => expect(startIdea).toHaveBeenCalled());
    expect(startIdea.mock.calls[0]).toEqual([4, false, false, true]);
  });

  it('cannot be started when the folder cannot be made, and says why', async () => {
    const { root, store, $$ } = await mount();
    await store.editIdea(2, { folder: '/nowhere/at/all' });
    card(root, 'ideas', 'Retry budget').click();
    await vi.waitFor(() => expect($$('.page .acts .btn').length).toBeGreaterThan(0));
    await vi.waitFor(() => expect(buttons(root)[0]!.disabled).toBe(true));
    expect(buttons(root)[0]!.title).toContain('inside your home folder');
    expect(root.querySelector('.page .fnote')!.textContent).toContain('inside your home folder');
  });

  it('is told about under the folder field while typing, and can still be saved', async () => {
    const addIdea = vi.fn(fixtureSource().addIdea);
    const { $ } = await mount({ ...fixtureSource(), addIdea });
    $('.btn.plus')!.click();
    await vi.waitFor(() => expect($('form.modal')).not.toBeNull());
    const type = (v: string) => {
      const f = $<HTMLInputElement>('.modal input[name="folder"]')!;
      f.value = v;
      f.dispatchEvent(new Event('input', { bubbles: true }));
    };
    expect($('.modal .fhint')).toBeNull();
    type('~/brand/new');
    await vi.waitFor(() => expect($('.modal .fhint')!.textContent).toBe('Folder does not exist yet; Start will create it'));
    type('/etc/none');
    await vi.waitFor(() => expect($('.modal .fhint')!.textContent).toContain('inside your home folder'));
    type('~/code/storefront');
    await vi.waitFor(() => expect($('.modal .fhint')).toBeNull());
    type('~/brand/new');
    $<HTMLInputElement>('.modal input[name="title"]')!.value = 'Saved anyway';
    $('form.modal')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(addIdea).toHaveBeenCalled());
    expect(addIdea.mock.calls[0]![0]).toMatchObject({ title: 'Saved anyway', folder: '~/brand/new' });
    await vi.waitFor(() => expect($('form.modal')).toBeNull());
  });

  it('shows no hint when the engine cannot check folders', async () => {
    const folderCheck = vi.fn(async () => { throw new Error('HTTP 404'); });
    const { $ } = await mount({ ...fixtureSource(), folderCheck });
    $('.btn.plus')!.click();
    await vi.waitFor(() => expect($('form.modal')).not.toBeNull());
    const f = $<HTMLInputElement>('.modal input[name="folder"]')!;
    f.value = '~/anything';
    f.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => expect(folderCheck).toHaveBeenCalled());
    expect($('.modal .fhint')).toBeNull();
  });
});

describe('the accent picker', () => {
  afterEach(() => { delete document.documentElement.dataset.accent; });

  it('is one button next to the theme switch, showing the current accent, with no swatches until opened', async () => {
    const { $, $$ } = await mount();
    const b = $<HTMLButtonElement>('.hd .accbtn')!;
    expect(b.tagName).toBe('BUTTON');
    expect(b.dataset.accent).toBe('vermilion');
    expect(b.getAttribute('aria-label')).toBe('Accent colour: Vermilion (original)');
    expect(b.getAttribute('aria-expanded')).toBe('false');
    expect($$('.hd .acc')).toHaveLength(0);
    expect($('.hd [aria-label="Theme"]')!.nextElementSibling).toBe($('.hd .accpick'));
  });

  it('opens eight named swatches in one popover, the chosen one marked', async () => {
    const { $, $$ } = await mount();
    $<HTMLButtonElement>('.hd .accbtn')!.click();
    await vi.waitFor(() => expect($('.hd .accpop')).not.toBeNull());
    const sw = $$<HTMLButtonElement>('.hd .accpop .acc');
    expect(sw.map((s) => s.getAttribute('aria-label'))).toEqual(['Vermilion (original)', 'Cobalt', 'Teal', 'Moss', 'Amber', 'Magenta', 'Violet', 'Graphite']);
    expect(sw.map((s) => s.title)).toEqual(sw.map((s) => s.getAttribute('aria-label')));
    expect(sw.map((s) => s.getAttribute('aria-pressed'))).toEqual(['true', ...Array(7).fill('false')]);
    expect($('.hd .accbtn')!.getAttribute('aria-expanded')).toBe('true');
  });

  it('picking a swatch sets the accent, closes the popover and returns the focus to the button', async () => {
    const { $, $$ } = await mount();
    $<HTMLButtonElement>('.hd .accbtn')!.click();
    await vi.waitFor(() => expect($('.hd .accpop')).not.toBeNull());
    $$<HTMLButtonElement>('.hd .accpop .acc').find((s) => s.getAttribute('aria-label') === 'Moss')!.click();
    await vi.waitFor(() => expect(document.documentElement.dataset.accent).toBe('moss'));
    await vi.waitFor(() => expect($('.hd .accpop')).toBeNull());
    expect($('.hd .accbtn')!.getAttribute('aria-label')).toBe('Accent colour: Moss');
    expect(document.activeElement).toBe($('.hd .accbtn'));
  });

  it('closes on Esc, with the focus back on the button, and on a click outside', async () => {
    const { $ } = await mount();
    const b = $<HTMLButtonElement>('.hd .accbtn')!;
    b.click();
    await vi.waitFor(() => expect($('.hd .accpop')).not.toBeNull());
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await vi.waitFor(() => expect($('.hd .accpop')).toBeNull());
    expect(document.activeElement).toBe(b);
    b.click();
    await vi.waitFor(() => expect($('.hd .accpop')).not.toBeNull());
    document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    await vi.waitFor(() => expect($('.hd .accpop')).toBeNull());
    expect(document.documentElement.dataset.accent).toBeUndefined();
  });

  it('stays open while the focus moves between its swatches and closes when Tab leaves it', async () => {
    const { $, $$ } = await mount();
    $<HTMLButtonElement>('.hd .accbtn')!.click();
    await vi.waitFor(() => expect($('.hd .accpop')).not.toBeNull());
    const [first, second] = $$<HTMLButtonElement>('.hd .accpop .acc');
    first!.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: second }));
    expect($('.hd .accpop')).not.toBeNull();
    second!.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: document.body }));
    await vi.waitFor(() => expect($('.hd .accpop')).toBeNull());
  });
});
