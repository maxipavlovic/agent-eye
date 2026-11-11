// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createStore } from './store';
import type { Board, Card, Column, ColumnId, Source } from './api/types';

function card(id: string, column: ColumnId): Card {
  return {
    id, agent: 'claude', agent_name: 'Claude', title: id, folder: null, repo: null, branch: null, column, auto_column: column, manual: false,
    last_activity_at: 1, started_at: 1, state: null, state_source: 'none', next: null, ai_status: null, recap: null,
    live_status: null, kind: 'interactive', streams: [], outputs: [], output_counts: {}, topics: [], topics_more: 0,
    resume_cmd: null, resumable: true, parent_id: null, child_count: 0, child_done: 0, prompt_count: 0, tokens: null,
  };
}

const ids: ColumnId[] = ['ideas', 'running', 'needs_you', 'waiting', 'paused', 'done'];
function boardOf(map: Partial<Record<ColumnId, string[]>>): Board {
  const columns: Column[] = ids.map((id) => ({ id, title: id, cards: (map[id] ?? []).map((c) => card(c, id)) }));
  return { columns, archive_count: 0, generated_at: 1 };
}
const where = (b: Board | null, id: string) => b?.columns.find((c) => c.cards.some((x) => x.id === id))?.id;

function source(over: Partial<Source> = {}): Source {
  return {
    name: 'live',
    board: async () => boardOf({ needs_you: ['a', 'b'], paused: ['c'] }),
    card: async (id) => ({ ...card(id, 'paused'), prompts: [], first_prompt: null, last_assistant_text: null, resume_mode: 'copy' }),
    archive: async () => ({ cards: [], total: 0, offset: 0 }),
    search: async () => ({ results: [] }),
    ideas: async () => ({ ideas: [] }),
    agents: async () => ({ agents: [{ id: 'claude', name: 'Claude Code', short: 'Claude', default: true }] }),
    move: async (id) => card(id, 'done'),
    setHidden: async (id) => card(id, 'paused'),
    conversation: async () => ({ messages: [], more: false, next_before: null }),
    resume: async () => ({ dry_run: false, mode: 'copy', command: 'x' }),
    addIdea: async () => card('idea:1', 'ideas'),
    editIdea: async () => card('idea:1', 'ideas'),
    startIdea: async () => ({ dry_run: false, mode: 'iterm', command: 'x', session_id: null, job: null }),
    folderCheck: async () => ({ state: 'ok', creatable: false, note: null }),
    subscribe: () => () => {},
    ...over,
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

let mm: ((e: { matches: boolean }) => void) | undefined;
beforeEach(() => {
  localStorage.clear();
  mm = undefined;
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline')));
  vi.stubGlobal('matchMedia', () => ({
    matches: false,
    addEventListener: (_: string, f: (e: { matches: boolean }) => void) => { mm = f; },
  }));
});
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('setHidden', () => {
  it('drops the card at once and keeps it gone after the refresh', async () => {
    let boards = 0;
    const hide = vi.fn(async (id: string) => card(id, 'paused'));
    const s = createStore(source({
      board: async () => (++boards === 1 ? boardOf({ needs_you: ['a', 'b'] }) : boardOf({ needs_you: ['b'] })),
      setHidden: hide,
    }));
    await tick();
    const p = s.setHidden('a', true);
    expect(where(s.board.value, 'a')).toBeUndefined();
    await p;
    expect(hide).toHaveBeenCalledWith('a', true);
    expect(where(s.board.value, 'b')).toBe('needs_you');
  });

  it('puts the card back and toasts when the server refuses', async () => {
    const s = createStore(source({ setHidden: async () => { throw new Error('nope'); } }));
    await tick();
    await s.setHidden('a', true);
    expect(where(s.board.value, 'a')).toBe('needs_you');
    expect(s.toasts.value.at(-1)).toMatchObject({ text: 'nope', tone: 'error' });
  });
});

describe('toast actions', () => {
  it('keeps the action and can be dismissed', async () => {
    const s = createStore(source());
    const run = vi.fn();
    const id = s.toast('Moved', 'info', { label: 'UNDO', run });
    expect(s.toasts.value.at(-1)!.action!.label).toBe('UNDO');
    s.dismissToast(id);
    expect(s.toasts.value).toHaveLength(0);
  });
});

describe('move', () => {
  it('moves optimistically, then takes the server board', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    let boards = 0;
    const s = createStore(source({
      board: async () => (++boards === 1 ? boardOf({ needs_you: ['a', 'b'] }) : boardOf({ done: ['a'], needs_you: ['b'] })),
      move: async (id) => { await gate; return card(id, 'done'); },
    }));
    await tick();
    const p = s.move('a', 'done');
    expect(where(s.board.value, 'a')).toBe('done');
    expect(s.board.value!.columns.find((c) => c.id === 'done')!.cards[0]!.manual).toBe(true);
    expect(s.needsYou.value).toBe(1);
    release();
    await p;
    expect(where(s.board.value, 'a')).toBe('done');
  });

  it('reverts and toasts an error when the server refuses', async () => {
    const s = createStore(source({ move: async () => { throw new Error('nope'); } }));
    await tick();
    await s.move('a', 'done');
    expect(where(s.board.value, 'a')).toBe('needs_you');
    expect(s.toasts.value.at(-1)).toMatchObject({ text: 'nope', tone: 'error' });
  });

  it('does not place locally for archive or auto', async () => {
    const mv = vi.fn(async (id: string) => card(id, 'done'));
    const s = createStore(source({ move: mv }));
    await tick();
    const p = s.move('a', 'archive');
    expect(where(s.board.value, 'a')).toBe('needs_you');
    await p;
    expect(mv).toHaveBeenCalledWith('a', 'archive');
  });
});

describe('search', () => {
  it('debounces and applies only the latest query', async () => {
    vi.useFakeTimers();
    const calls: string[] = [];
    const resolvers: Record<string, (v: { results: never[] }) => void> = {};
    const s = createStore(source({
      search: (q) => { calls.push(q); return new Promise((r) => { resolvers[q] = r as never; }); },
    }));
    s.search('cat');
    s.search('catalog');
    expect(s.searching.value).toBe(true);
    await vi.advanceTimersByTimeAsync(200);
    expect(calls).toEqual(['catalog']);
    resolvers['catalog']!({ results: [] });
    await vi.advanceTimersByTimeAsync(0);
    expect(s.results.value).toEqual([]);
    expect(s.searching.value).toBe(false);
  });

  it('ignores an older answer that arrives after a newer one', async () => {
    vi.useFakeTimers();
    const hit = (t: string) => ({ results: [{ card: card(t, 'paused'), snippet: t, score: 0 }] });
    const resolvers: Record<string, (v: ReturnType<typeof hit>) => void> = {};
    const s = createStore(source({
      search: (q) => new Promise((r) => { resolvers[q] = r as never; }),
    }));
    s.search('one');
    await vi.advanceTimersByTimeAsync(200);
    s.search('two');
    await vi.advanceTimersByTimeAsync(200);
    resolvers['two']!(hit('two'));
    await vi.advanceTimersByTimeAsync(0);
    resolvers['one']!(hit('one'));
    await vi.advanceTimersByTimeAsync(0);
    expect(s.results.value!.map((h) => h.snippet)).toEqual(['two']);
  });

  it('clears results for an empty query and cancels a pending search', async () => {
    vi.useFakeTimers();
    const search = vi.fn(async () => ({ results: [] }));
    const s = createStore(source({ search }));
    s.search('abc');
    s.search('  ');
    await vi.advanceTimersByTimeAsync(300);
    expect(search).not.toHaveBeenCalled();
    expect(s.results.value).toBeNull();
    expect(s.searching.value).toBe(false);
  });
});

describe('theme', () => {
  it('follows the OS in auto, changes with it, and sets data-theme', async () => {
    const s = createStore(source());
    expect(s.themePref.value).toBe('auto');
    expect(s.theme.value).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    mm!({ matches: true });
    expect(s.theme.value).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('an explicit choice wins and is remembered', () => {
    const s = createStore(source());
    s.setTheme('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(localStorage.getItem('eye.theme')).toBe('dark');
    expect(createStore(source()).themePref.value).toBe('dark');
    s.setTheme('light');
    mm!({ matches: true });
    expect(s.theme.value).toBe('light');
  });
});

describe('accent', () => {
  beforeEach(() => { delete document.documentElement.dataset.accent; });

  it('is vermilion, with no attribute, until one is chosen', () => {
    const s = createStore(source());
    expect(s.accent.value).toBe('vermilion');
    expect(document.documentElement.dataset.accent).toBeUndefined();
  });

  it('a choice sets data-accent, is remembered, and vermilion takes the attribute away again', () => {
    const s = createStore(source());
    s.setAccent('cobalt');
    expect(document.documentElement.dataset.accent).toBe('cobalt');
    expect(localStorage.getItem('eye.accent')).toBe('cobalt');
    expect(createStore(source()).accent.value).toBe('cobalt');
    s.setAccent('vermilion');
    expect(document.documentElement.dataset.accent).toBeUndefined();
    expect(createStore(source()).accent.value).toBe('vermilion');
  });

  it('reads an unknown saved value as vermilion', () => {
    localStorage.setItem('eye.accent', 'chartreuse');
    const s = createStore(source());
    expect(s.accent.value).toBe('vermilion');
    expect(document.documentElement.dataset.accent).toBeUndefined();
  });

  it('is sent to the engine with the other board settings', async () => {
    const f = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) });
    vi.stubGlobal('fetch', f);
    createStore(source()).setAccent('teal');
    const call = f.mock.calls.find((c) => c[0] === '/api/prefs' && c[1]?.method === 'POST')!;
    expect(JSON.parse(call[1].body)).toEqual({ 'eye.accent': 'teal' });
  });
});

describe('agents', () => {
  it('lists the enabled agents', async () => {
    const s = createStore(source());
    expect(s.agents.value).toEqual([]);
    await tick();
    expect(s.agents.value).toEqual([{ id: 'claude', name: 'Claude Code', short: 'Claude', default: true }]);
  });

  it('stays empty when the engine has no agent list', async () => {
    const s = createStore(source({ agents: async () => { throw new Error('404'); } }));
    await tick();
    expect(s.agents.value).toEqual([]);
    expect(s.toasts.value).toHaveLength(0);
  });
});

describe('connection and errors', () => {
  it('shows the source state and a load error', async () => {
    let push!: (s: 'live') => void;
    const s = createStore(source({
      board: async () => { throw new Error('Not signed in'); },
      subscribe: (_c, st) => { push = st as never; return () => {}; },
    }));
    expect(s.connection.value).toBe('connecting');
    push('live');
    expect(s.connection.value).toBe('live');
    await tick();
    expect(s.error.value).toBe('Not signed in');
  });
});

describe('copy mode', () => {
  const copyPlan = (note?: string) => async () => ({ dry_run: false, mode: 'copy', command: 'cd /w && codex resume 1', ...(note ? { note } : {}) });

  it('also copies the command in the browser, whatever the daemon could do', async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const s = createStore(source({ resume: copyPlan('no clipboard tool; here is the command: cd /w && codex resume 1') }));
    await s.resume('a');
    expect(writeText).toHaveBeenCalledWith('cd /w && codex resume 1');
    expect(s.toasts.value.at(-1)!.text).toBe('Command copied');
    await s.resume('a', true);
    expect(writeText).toHaveBeenCalledTimes(2);
  });

  it('says what the daemon said when the page may not copy, and names no terminal or system', async () => {
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn(async () => { throw new Error('denied'); }) } });
    const s = createStore(source({ resume: copyPlan('command copied to the clipboard') }));
    await s.resume('a');
    expect(s.toasts.value.at(-1)!.text).toBe('command copied to the clipboard');
    const bare = createStore(source({ resume: copyPlan() }));
    await bare.resume('a');
    expect(bare.toasts.value.at(-1)!.text).toBe('Copy the command from the panel');
    vi.stubGlobal('navigator', {});
    const none = createStore(source({ resume: copyPlan() }));
    await none.resume('a');
    expect(none.toasts.value.at(-1)!.text).toBe('Copy the command from the panel');
    for (const t of [...s.toasts.value, ...bare.toasts.value, ...none.toasts.value]) expect(t.text).not.toMatch(/iterm|copy$/i);
  });

  it('copies the command of a start in the foreground and not that of one in the background', async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const startIdea = vi.fn(async (_id: number, background: boolean) => ({
      dry_run: false, mode: 'copy', command: "cd '/w' && codex 'go'", session_id: null, job: background ? 'ab12' : null,
    }));
    const s = createStore(source({ startIdea }));
    const idea = { ...card('idea:1', 'ideas'), idea_id: 1 };
    await s.startIdea(idea, false);
    expect(writeText).toHaveBeenCalledWith("cd '/w' && codex 'go'");
    expect(s.toasts.value.at(-1)!.text).toBe('Command copied');
    writeText.mockClear();
    await s.startIdea(idea, true);
    expect(writeText).not.toHaveBeenCalled();
    expect(s.toasts.value.at(-1)!.text).toBe('Started in the background (ab12)');
  });

  it('asks for the folder to be made only when it is missing and may be made', async () => {
    const startIdea = vi.fn(async () => ({ dry_run: false, mode: 'iterm', command: 'x', session_id: null, job: null }));
    const s = createStore(source({ startIdea }));
    const idea = { ...card('idea:1', 'ideas'), idea_id: 1 };
    await s.startIdea(idea, false);
    expect(startIdea).toHaveBeenLastCalledWith(1, false, false, false);
    await s.startIdea({ ...idea, folder_state: 'ok' }, false);
    expect(startIdea).toHaveBeenLastCalledWith(1, false, false, false);
    await s.startIdea({ ...idea, folder_state: 'missing', folder_creatable: true }, false);
    expect(startIdea).toHaveBeenLastCalledWith(1, false, false, true);
    await s.startIdea({ ...idea, folder_state: 'missing', folder_creatable: false }, true);
    expect(startIdea).toHaveBeenLastCalledWith(1, true, false, false);
  });

  it('opens a tab without naming the terminal when the daemon said nothing', async () => {
    const s = createStore(source());
    await s.startIdea({ ...card('idea:1', 'ideas'), idea_id: 1 }, false);
    expect(s.toasts.value.at(-1)!.text).toBe('Starting in a new terminal');
  });
});

describe('fork and conversation', () => {
  it('forks through the resume call and says so in the toast', async () => {
    const resume = vi.fn(async () => ({ dry_run: false, mode: 'iterm', command: 'x' }));
    const s = createStore(source({ resume }));
    await s.resume('a', true);
    expect(resume).toHaveBeenCalledWith('a', false, true);
    expect(s.toasts.value.at(-1)!.text).toBe('Forking in a new terminal');
    await s.resume('a');
    expect(resume).toHaveBeenLastCalledWith('a', false, false);
    expect(s.toasts.value.at(-1)!.text).toBe('Opening in a new terminal');
  });

  it('passes the page cursor and fails quietly', async () => {
    const conversation = vi.fn(async () => ({ messages: [], more: true, next_before: 4 }));
    const s = createStore(source({ conversation }));
    expect(await s.conversation('a', 7, 5)).toEqual({ messages: [], more: true, next_before: 4 });
    expect(conversation).toHaveBeenCalledWith('a', 5, 7);
    const bad = createStore(source({ conversation: async () => { throw new Error('404'); } }));
    expect(await bad.conversation('a')).toBeNull();
    expect(bad.toasts.value).toHaveLength(0);
  });
});
