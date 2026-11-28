/**
 * The engine's event stream (`GET /api/events`), read with `fetch`: an `EventSource` cannot send the
 * token header. It tells the page when the data changed. The stream is plain server-sent events:
 * blocks of `event:` and `data:` lines, a blank line after each, and `:` comment lines to keep it alive.
 */
import type { Connection } from '../api/types';
import { apiFetch, forgetToken, token } from './auth';

/** Consecutive failures before the badge says offline. */
const OFFLINE_AFTER = 3;
/** The wait before asking again after the stream ended or failed (what a browser's EventSource does). */
const RETRY_MS = 3000;

/** The event names in `text`, one per block that ends in a blank line; the rest of the text is returned too. */
export function parseEvents(text: string): { names: string[]; rest: string } {
  const blocks = text.replaceAll('\r\n', '\n').split('\n\n');
  const rest = blocks.pop() ?? '';
  const names = blocks.map((b) => {
    const line = b.split('\n').find((l) => l.startsWith('event:'));
    return line ? line.slice('event:'.length).trim() : 'message';
  });
  return { names, rest };
}

async function readStream(body: ReadableStream<Uint8Array>, on: (name: string) => void): Promise<void> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let text = '';
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    text += decoder.decode(value, { stream: true });
    const { names, rest } = parseEvents(text);
    text = rest;
    names.forEach(on);
  }
}

/**
 * Call `onChange` for each `board-changed` event and report the connection; asks again after a pause
 * when the stream ends. Returns the function that stops it.
 */
export function listen(onChange: () => void, onState: (s: Connection) => void): () => void {
  const ctl = new AbortController();
  let errors = 0;
  const failed = () => { errors++; onState(errors >= OFFLINE_AFTER ? 'offline' : 'connecting'); };
  const pause = () => new Promise<void>((ok) => {
    const t = setTimeout(ok, RETRY_MS);
    ctl.signal.addEventListener('abort', () => { clearTimeout(t); ok(); }, { once: true });
  });
  const run = async () => {
    onState('connecting');
    while (!ctl.signal.aborted) {
      if (!token()) { onState('offline'); return; }
      try {
        const res = await apiFetch('/api/events', { signal: ctl.signal, headers: { accept: 'text/event-stream' } });
        if (res.status === 401) { forgetToken(); onState('offline'); return; }
        if (!res.ok || !res.body) throw new Error(`HTTP ${res.status}`);
        errors = 0;
        onState('live');
        await readStream(res.body, (name) => { if (name === 'board-changed') onChange(); });
        failed();
      } catch {
        if (ctl.signal.aborted) return;
        failed();
      }
      await pause();
    }
  };
  void run();
  return () => ctl.abort();
}
