// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { adoptToken, apiFetch, forgetToken, linkManifest, token } from './auth';

const TOKEN = 'b'.repeat(64);

function at(url: string) {
  history.replaceState(null, '', url);
}

beforeEach(() => {
  localStorage.clear();
  forgetToken();
  document.head.innerHTML = '';
  at('/');
});
afterEach(() => vi.unstubAllGlobals());

describe('the token from the link', () => {
  it('is kept, and taken out of the address with every other part of it left', () => {
    at(`/?t=${TOKEN}&look=iris#top`);
    adoptToken();
    expect(token()).toBe(TOKEN);
    expect(localStorage.getItem('eye.token')).toBe(TOKEN);
    expect(location.pathname + location.search + location.hash).toBe('/?look=iris#top');
    at(`/?t=${TOKEN}`);
    adoptToken();
    expect(location.search).toBe('');
  });

  it('is found again by a page opened without it, which is how the installed app comes back after a restart', () => {
    at(`/?t=${TOKEN}`);
    adoptToken();
    forgetToken();
    localStorage.setItem('eye.token', TOKEN);
    at('/');
    adoptToken();
    expect(token()).toBe(TOKEN);
    expect(location.search).toBe('');
  });

  it('a page opened with no link and none kept has none', () => {
    adoptToken();
    expect(token()).toBeNull();
  });

  it('is held for the page when the browser will not store it', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); });
    at(`/?t=${TOKEN}`);
    adoptToken();
    expect(token()).toBe(TOKEN);
    vi.restoreAllMocks();
  });

  it('is dropped when forgotten', () => {
    localStorage.setItem('eye.token', TOKEN);
    forgetToken();
    expect(token()).toBeNull();
    expect(localStorage.getItem('eye.token')).toBeNull();
  });

  it('is never put in a cookie', () => {
    at(`/?t=${TOKEN}`);
    adoptToken();
    linkManifest();
    expect(document.cookie).toBe('');
  });
});

describe('apiFetch', () => {
  it('sends the token in the header and no cookies, keeping the caller\'s own options', async () => {
    localStorage.setItem('eye.token', TOKEN);
    const f = vi.fn().mockResolvedValue(new Response('{}'));
    vi.stubGlobal('fetch', f);
    await apiFetch('/api/board', { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' });
    const [url, init] = f.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/board');
    expect(init).toMatchObject({ method: 'POST', body: '{}', credentials: 'omit' });
    const h = new Headers(init.headers);
    expect(h.get('X-Eye-Token')).toBe(TOKEN);
    expect(h.get('content-type')).toBe('application/json');
  });

  it('sends no header without a token', async () => {
    const f = vi.fn().mockResolvedValue(new Response('{}'));
    vi.stubGlobal('fetch', f);
    await apiFetch('/api/board');
    expect(new Headers((f.mock.calls[0]![1] as RequestInit).headers).has('X-Eye-Token')).toBe(false);
  });
});

describe('the app manifest', () => {
  it('is linked with the token in its address, because the browser fetches it and cannot send a header', () => {
    localStorage.setItem('eye.token', TOKEN);
    linkManifest();
    linkManifest();
    const links = document.querySelectorAll('link[rel=manifest]');
    expect(links).toHaveLength(1);
    expect(links[0]!.getAttribute('href')).toBe(`/manifest.webmanifest?t=${TOKEN}`);
  });

  it('is not linked without a token', () => {
    linkManifest();
    expect(document.querySelector('link[rel=manifest]')).toBeNull();
  });
});
