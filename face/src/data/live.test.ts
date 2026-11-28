// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { liveSource } from './live';
import { token } from './auth';

const TOKEN = 'a'.repeat(64);

/** Every call: no cookies, the token in the header. */
const SIGNED = expect.objectContaining({ credentials: 'omit' });
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

beforeEach(() => { localStorage.clear(); localStorage.setItem('eye.token', TOKEN); });
afterEach(() => { vi.unstubAllGlobals(); });

describe('liveSource requests', () => {
  it('GETs the board same-origin', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { columns: [], archive_count: 0, generated_at: 1 }));
    vi.stubGlobal('fetch', f);
    const b = await liveSource().board();
    expect(b.archive_count).toBe(0);
    expect(f).toHaveBeenCalledWith('/api/board', SIGNED);
  });

  it('sends the token in a header on every call, reads and writes alike, and never a cookie', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { id: 'x' }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    await s.board();
    await s.move('abc', 'done');
    for (const [, init] of f.mock.calls as Array<[string, RequestInit]>) {
      expect(new Headers(init.headers).get('X-Eye-Token')).toBe(TOKEN);
      expect(init.credentials).toBe('omit');
    }
    // the POST keeps its own header too
    expect(new Headers((f.mock.calls[1]![1] as RequestInit).headers).get('content-type')).toBe('application/json');
    // and the token is never in an address
    for (const [url] of f.mock.calls as Array<[string]>) expect(url).not.toContain(TOKEN);
  });

  it('sends confirm: true with a stop or remove, and not with an ignore', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { ok: true }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    await s.looseAct('stop', 'proc:5:1');
    await s.looseAct('remove', 'worktree:/w');
    await s.looseAct('ignore', 'proc:5:1');
    const body = (i: number) => JSON.parse((f.mock.calls[i]![1] as { body: string }).body);
    expect(f.mock.calls[0]![0]).toBe('/api/loose/stop');
    expect(body(0)).toEqual({ key: 'proc:5:1', confirm: true });
    expect(body(1)).toEqual({ key: 'worktree:/w', confirm: true });
    expect(body(2)).toEqual({ key: 'proc:5:1' });
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
    expect(f).toHaveBeenCalledWith('/api/agents', SIGNED);
  });

  it('asks for usage over a number of days', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { days: 30, rows: [] }));
    vi.stubGlobal('fetch', f);
    const r = await liveSource().usage(30);
    expect(r.days).toBe(30);
    expect(f).toHaveBeenCalledWith('/api/usage?days=30', SIGNED);
  });

  it('asks for the steering clock over a number of weeks', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { enabled: true, hours: [] }));
    vi.stubGlobal('fetch', f);
    const r = await liveSource().clock(8);
    expect(r.enabled).toBe(true);
    expect(f).toHaveBeenCalledWith('/api/clock?weeks=8', SIGNED);
  });

  it('asks for the daily series', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { days: 14, rows: [] }));
    vi.stubGlobal('fetch', f);
    await liveSource().usage(14, 'model');
    expect(f).toHaveBeenCalledWith('/api/usage?days=14&series=model', SIGNED);
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

  it('posts a reminder to the card, an idea card included', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { id: 'idea:1' }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    await s.remind('idea:1', { when: 'mon', note: 'by Friday' });
    expect(f.mock.calls[0]![0]).toBe('/api/cards/idea%3A1/remind');
    expect(JSON.parse(f.mock.calls[0]![1].body)).toEqual({ when: 'mon', note: 'by Friday' });
    await s.remind('abc', { clear: true });
    expect(f.mock.calls[1]![0]).toBe('/api/cards/abc/remind');
    expect(JSON.parse(f.mock.calls[1]![1].body)).toEqual({ clear: true });
  });

  it('reads and writes the desktop notice setting, and an older engine answers 404', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { notify: true, available: true }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    expect(await s.remindSettings()).toEqual({ notify: true, available: true });
    expect(f).toHaveBeenCalledWith('/api/remind/settings', SIGNED);
    await s.setRemindSettings({ notify: false });
    expect(f.mock.calls[1]![0]).toBe('/api/remind/settings');
    expect(f.mock.calls[1]![1]).toMatchObject({ method: 'POST', body: '{"notify":false}' });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json(404, { error: 'not found' })));
    await expect(s.remindSettings()).rejects.toMatchObject({ status: 404 });
  });

  it('reads the settings and posts only the keys that changed, and the engine\'s refusal is the message', async () => {
    const f = vi.fn().mockResolvedValue(json(200, { settings: {} }));
    vi.stubGlobal('fetch', f);
    const s = liveSource();
    await s.settings();
    expect(f).toHaveBeenCalledWith('/api/settings', SIGNED);
    await s.saveSettings({ 'night.parallel': 3 });
    expect(f.mock.calls[1]![0]).toBe('/api/settings');
    expect(f.mock.calls[1]![1]).toMatchObject({ method: 'POST', body: '{"night.parallel":3}' });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json(400, { error: 'night.parallel: give a number from 1 to 8' })));
    await expect(s.saveSettings({ 'night.parallel': 99 })).rejects.toThrow('give a number from 1 to 8');
  });

  it('turns {error} into the message', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json(400, { error: 'Workspace not trusted' })));
    await expect(liveSource().resume('a', false)).rejects.toThrow('Workspace not trusted');
  });

  it('explains a 401, and forgets a token the engine refused', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('missing token', { status: 401 })));
    await expect(liveSource().board()).rejects.toThrow(/eye board/);
    expect(token()).toBeNull();
  });

  it('without a token says how to open the board and asks nothing', async () => {
    localStorage.clear();
    const f = vi.fn();
    vi.stubGlobal('fetch', f);
    await expect(liveSource().board()).rejects.toMatchObject({ status: 401, message: expect.stringMatching(/`eye board`.*link/) });
    await expect(liveSource().exportSession('a')).rejects.toMatchObject({ status: 401 });
    expect(f).not.toHaveBeenCalled();
  });

  it('fetches an export with the token and names the file as the engine does', async () => {
    const f = vi.fn().mockResolvedValue(new Response('<html>page</html>', { status: 200, headers: { 'content-disposition': 'attachment; filename="eye-abc-fix-it.html"' } }));
    vi.stubGlobal('fetch', f);
    const file = await liveSource().exportSession('a b');
    expect(f.mock.calls[0]![0]).toBe('/api/sessions/a%20b/export?format=html');
    expect(new Headers((f.mock.calls[0]![1] as RequestInit).headers).get('X-Eye-Token')).toBe(TOKEN);
    expect(file.name).toBe('eye-abc-fix-it.html');
    expect(await file.blob.text()).toBe('<html>page</html>');
    // no name from the engine: one of our own
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('x', { status: 200 })));
    expect((await liveSource().exportSession('abc')).name).toBe('eye-abc.html');
  });

  it('turns a refused export into the engine\'s message', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json(404, { error: 'no such session' })));
    await expect(liveSource().exportSession('zzz')).rejects.toMatchObject({ status: 404, message: 'no such session' });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('boom', { status: 500 })));
    await expect(liveSource().exportSession('zzz')).rejects.toMatchObject({ status: 500, message: 'HTTP 500' });
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
  // the event stream itself is tested in events.test.ts; here only the page's own refresh
  beforeEach(() => { vi.stubGlobal('fetch', vi.fn().mockReturnValue(new Promise(() => {}))); });

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
