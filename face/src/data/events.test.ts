// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { listen, parseEvents } from './events';
import type { Connection } from '../api/types';

const TOKEN = 'c'.repeat(64);

/** A response whose body the test feeds chunk by chunk. */
function stream() {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  const body = new ReadableStream<Uint8Array>({ start(c) { controller = c; } });
  const enc = new TextEncoder();
  return {
    response: new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
    send: (text: string) => controller.enqueue(enc.encode(text)),
    end: () => controller.close(),
  };
}

const settle = async (n = 5) => { for (let i = 0; i < n; i++) await Promise.resolve(); await vi.advanceTimersByTimeAsync(0); };

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  localStorage.setItem('eye.token', TOKEN);
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('parseEvents', () => {
  it('reads the event of each finished block and keeps the unfinished rest', () => {
    expect(parseEvents('event: board-changed\ndata: 1\n\n:\n\nevent: board-changed\ndata: 1\n\nevent: boa'))
      .toEqual({ names: ['board-changed', 'message', 'board-changed'], rest: 'event: boa' });
    expect(parseEvents('event:board-changed\r\ndata: 1\r\n\r\n').names).toEqual(['board-changed']);
    expect(parseEvents('')).toEqual({ names: [], rest: '' });
  });
});

describe('listen', () => {
  it('asks with the token in a header, reports live, and calls onChange for a board-changed event only', async () => {
    const s = stream();
    const f = vi.fn().mockResolvedValue(s.response);
    vi.stubGlobal('fetch', f);
    const states: Connection[] = [];
    const onChange = vi.fn();
    const stop = listen(onChange, (c) => states.push(c));
    await settle();
    const [url, init] = f.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/events');
    expect(new Headers(init.headers).get('X-Eye-Token')).toBe(TOKEN);
    expect(init.credentials).toBe('omit');
    expect(states).toEqual(['connecting', 'live']);
    s.send(':\n\n');
    await settle();
    expect(onChange).not.toHaveBeenCalled();
    // an event split over two chunks is still one event
    s.send('event: board-ch');
    await settle();
    s.send('anged\ndata: 1\n\n');
    await settle();
    expect(onChange).toHaveBeenCalledTimes(1);
    stop();
  });

  it('asks again after the stream ends, goes offline after three failures in a row, and is live again when one works', async () => {
    const first = stream();
    const second = stream();
    const f = vi.fn()
      .mockResolvedValueOnce(first.response)
      .mockRejectedValueOnce(new TypeError('down'))
      .mockRejectedValueOnce(new TypeError('down'))
      .mockResolvedValue(second.response);
    vi.stubGlobal('fetch', f);
    const states: Connection[] = [];
    const stop = listen(() => {}, (c) => states.push(c));
    await settle();
    first.end();
    await settle();
    expect(states.at(-1)).toBe('connecting');
    // the ended stream was the first failure, the two refused asks the next ones
    for (let i = 0; i < 2; i++) await vi.advanceTimersByTimeAsync(3000);
    expect(states).toEqual(['connecting', 'live', 'connecting', 'connecting', 'offline']);
    await vi.advanceTimersByTimeAsync(3000);
    await settle();
    expect(states.at(-1)).toBe('live');
    stop();
  });

  it('stops asking, and aborts what is open, when stopped', async () => {
    const s = stream();
    const f = vi.fn().mockResolvedValue(s.response);
    vi.stubGlobal('fetch', f);
    const stop = listen(() => {}, () => {});
    await settle();
    const signal = (f.mock.calls[0]![1] as RequestInit).signal!;
    expect(signal.aborted).toBe(false);
    stop();
    expect(signal.aborted).toBe(true);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(f).toHaveBeenCalledTimes(1);
  });

  it('a token the engine refuses is forgotten and the stream is not asked for again', async () => {
    const f = vi.fn().mockResolvedValue(new Response('missing token', { status: 401 }));
    vi.stubGlobal('fetch', f);
    const states: Connection[] = [];
    listen(() => {}, (c) => states.push(c));
    await settle();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(f).toHaveBeenCalledTimes(1);
    expect(states).toEqual(['connecting', 'offline']);
    expect(localStorage.getItem('eye.token')).toBeNull();
  });

  it('without a token it asks nothing and says offline', async () => {
    localStorage.clear();
    const f = vi.fn();
    vi.stubGlobal('fetch', f);
    const states: Connection[] = [];
    listen(() => {}, (c) => states.push(c));
    await settle();
    expect(f).not.toHaveBeenCalled();
    expect(states).toEqual(['connecting', 'offline']);
  });
});
