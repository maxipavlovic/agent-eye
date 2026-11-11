// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { liveSource } from './live';

class FakeES {
  static last: FakeES;
  listeners: Record<string, Array<() => void>> = {};
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(readonly url: string) { FakeES.last = this; }
  addEventListener(t: string, f: () => void) { (this.listeners[t] ??= []).push(f); }
  close() { this.closed = true; }
}

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

beforeEach(() => { vi.stubGlobal('EventSource', FakeES); });
afterEach(() => { vi.unstubAllGlobals(); });

describe('liveSource requests', () => {
  it('GETs the board same-origin', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { columns: [], archive_count: 0, generated_at: 1 }));
    vi.stubGlobal('fetch', f);
    const b = await liveSource().board();
    expect(b.archive_count).toBe(0);
    expect(f).toHaveBeenCalledWith('/api/board', { credentials: 'same-origin' });
  });

  it('encodes ids and queries', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { results: [] }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    await s.search('catalog migration & more');
    expect(f.mock.calls[0]![0]).toBe('/api/search?q=catalog%20migration%20%26%20more');
    await s.card('idea:3');
    expect(f.mock.calls[1]![0]).toBe('/api/cards/idea%3A3');
  });

  it('POSTs JSON bodies', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { id: 'x' }));
    vi.stubGlobal('fetch', f);
    await liveSource().move('abc', 'done');
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe('/api/cards/abc/column');
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body)).toEqual({ column: 'done' });
    await liveSource().setHidden('abc', true);
    const [hurl, hinit] = f.mock.calls[1]!;
    expect(hurl).toBe('/api/cards/abc/hidden');
    expect(JSON.parse(hinit.body)).toEqual({ hidden: true });
    await liveSource().startIdea(4, true, false);
    expect(JSON.parse(f.mock.calls[2]![1].body)).toEqual({ background: true, dry_run: false });
    await liveSource().startIdea(4, false, false, true);
    expect(JSON.parse(f.mock.calls[3]![1].body)).toEqual({ background: false, dry_run: false, create_folder: true });
  });

  it('asks the engine what a typed folder is', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { state: 'missing', creatable: true, note: 'x' }));
    vi.stubGlobal('fetch', f);
    expect(await liveSource().folderCheck('~/a b&c')).toEqual({ state: 'missing', creatable: true, note: 'x' });
    expect(f.mock.calls[0]![0]).toBe('/api/folder?path=~%2Fa%20b%26c');
  });

  it('asks for conversation pages and forks through the resume body', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { messages: [], more: false, next_before: null }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    await s.conversation('a b', 10, null);
    expect(f.mock.calls[0]![0]).toBe('/api/cards/a%20b/conversation?limit=10');
    await s.conversation('a', 10, 24);
    expect(f.mock.calls[1]![0]).toBe('/api/cards/a/conversation?limit=10&before=24');
    await s.resume('a', false, true);
    expect(JSON.parse(f.mock.calls[2]![1].body)).toEqual({ dry_run: false, fork: true });
    await s.resume('a', false);
    expect(JSON.parse(f.mock.calls[3]![1].body)).toEqual({ dry_run: false });
  });

  it('lists the enabled agents', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { agents: [{ id: 'claude', name: 'Claude Code', short: 'Claude', default: true }] }));
    vi.stubGlobal('fetch', f);
    const r = await liveSource().agents();
    expect(r.agents[0]!.short).toBe('Claude');
    expect(f).toHaveBeenCalledWith('/api/agents', { credentials: 'same-origin' });
  });

  it('sends the idea agent when one is chosen', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { id: 'idea:1' }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    await s.addIdea({ title: 'Spike', agent: 'codex' });
    expect(JSON.parse(f.mock.calls[0]![1].body)).toEqual({ title: 'Spike', agent: 'codex' });
    await s.editIdea(3, { agent: 'claude' });
    expect(f.mock.calls[1]![0]).toBe('/api/ideas/3');
    expect(JSON.parse(f.mock.calls[1]![1].body)).toEqual({ agent: 'claude' });
  });

  it('turns {error} into the message', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json(400, { error: 'Workspace not trusted' })));
    await expect(liveSource().resume('a', false)).rejects.toThrow('Workspace not trusted');
  });

  it('explains a 401', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('missing token', { status: 401 })));
    await expect(liveSource().board()).rejects.toThrow(/eye board/);
  });

  it('reports an unreachable server', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('failed')));
    await expect(liveSource().board()).rejects.toThrow(/Cannot reach eye/);
  });

  it('falls back to the status for non-JSON errors', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('boom', { status: 500 })));
    await expect(liveSource().board()).rejects.toThrow('HTTP 500');
  });
});

describe('liveSource.subscribe', () => {
  it('maps events to onChange and connection states', () => {
    const onChange = vi.fn();
    const states: string[] = [];
    const off = liveSource().subscribe(onChange, (s) => states.push(s));
    const es = FakeES.last;
    expect(es.url).toBe('/api/events');
    expect(states).toEqual(['connecting']);
    es.onopen!();
    es.listeners['board-changed']![0]!();
    expect(onChange).toHaveBeenCalledTimes(1);
    es.onerror!();
    es.onerror!();
    es.onerror!();
    es.onopen!();
    expect(states).toEqual(['connecting', 'live', 'connecting', 'connecting', 'offline', 'live']);
    off();
    expect(es.closed).toBe(true);
  });

  it('refreshes when the tab becomes visible, and stops after unsubscribe', () => {
    const onChange = vi.fn();
    const off = liveSource().subscribe(onChange, () => {});
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    expect(onChange).toHaveBeenCalledTimes(1);
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    expect(onChange).toHaveBeenCalledTimes(1);
    off();
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    expect(onChange).toHaveBeenCalledTimes(1);
  });
});
