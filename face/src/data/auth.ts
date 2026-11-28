/**
 * The board's token. The link `eye board` prints carries it as `?t=`. The page keeps it in
 * localStorage, which belongs to one scheme, host and port: no other server on 127.0.0.1 can read it,
 * and the board installed as an app still has it after a restart. It takes `t` out of the address and
 * sends the token as `X-Eye-Token` with every call. It is never put in a cookie: a cookie goes to every
 * server on 127.0.0.1 whatever its port.
 */

const KEY = 'eye.token';
/** Where the token is held when the browser will not store it: it lasts until the page is closed. */
let held: string | null = null;

function stored(): string | null {
  try { return localStorage.getItem(KEY); } catch { return null; }
}

/** The token this page has, or null when it was never opened with the link. */
export function token(): string | null {
  return stored() ?? held;
}

/** Take the token from the address (`?t=`), keep it, and drop it from the address and the history entry. */
export function adoptToken(): void {
  const url = new URL(location.href);
  const t = url.searchParams.get('t');
  if (!t) return;
  held = t;
  try { localStorage.setItem(KEY, t); } catch { /* blocked storage: this page still works */ }
  url.searchParams.delete('t');
  history.replaceState(history.state, '', `${url.pathname}${url.search}${url.hash}`);
}

/** The engine did not accept the token: drop it, so the page says how to get a good one. */
export function forgetToken(): void {
  held = null;
  try { localStorage.removeItem(KEY); } catch { /* blocked storage */ }
}

/** What to say when there is no token, or the engine refused it. */
export const NO_ACCESS = 'No access yet: run `eye board` in a terminal and open the link it prints.';

/** `fetch` for the engine's API: the token in a header, and no cookies sent. */
export function apiFetch(path: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  const t = token();
  if (t) headers.set('X-Eye-Token', t);
  return fetch(path, { ...init, headers, credentials: 'omit' });
}

/** The manifest of the installable app is fetched by the browser, which cannot send a header: it takes the token in its address. */
export function linkManifest(): void {
  const t = token();
  if (!t || document.querySelector('link[rel=manifest]')) return;
  const link = document.createElement('link');
  link.rel = 'manifest';
  link.href = `/manifest.webmanifest?t=${encodeURIComponent(t)}`;
  document.head.appendChild(link);
}
