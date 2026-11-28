// What every test shares: a fresh fixture home and a real `agent-eye serve` on it, per test, so
// no test sees what another changed. Nothing here touches the real home of whoever runs it:
// HOME, EYE_HOME and EYE_CLAUDE_DIR are temporary folders and PATH holds only fake agents.
import { test as base, expect, type Page, type Request } from '@playwright/test';
import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export { expect };

/** The instant the fixture is made for: 2026-11-20 12:00:00 UTC. The server clock and the browser clock are both set to it. */
export const NOW = 1_795_176_000_000;

const here = dirname(fileURLToPath(import.meta.url));
const target = resolve(here, '../target/e2e-linux/debug');
const EYE = process.env.EYE_BIN ?? join(target, 'agent-eye');
const FIXTURE = process.env.EYE_FIXTURE_BIN ?? join(target, 'examples/fixture_home');

/** The fixture's sessions, by the number in their ids. */
export const sid = (n: number) => `e2e00000-0000-4000-8000-${String(n).padStart(12, '0')}`;

const FAKE_AGENT = '#!/bin/sh\necho "fake agent: not run" >&2\nexit 1\n';

export interface Eye {
  dir: string;
  /** `http://127.0.0.1:<port>` */
  base: string;
  token: string;
  /** The board's address with the token: the page keeps the token and drops it from the address. */
  url: string;
  env: NodeJS.ProcessEnv;
  /** GET or POST a JSON route. */
  api: (path: string, body?: unknown) => Promise<any>;
  /** Run the CLI on the same home. */
  cli: (...args: string[]) => string;
  /** Store a "left behind" item for a process the test started; returns its key. */
  addProcess: (pid: number) => string;
}

function startServer(dir: string, env: NodeJS.ProcessEnv): Promise<{ child: ChildProcess; url: string }> {
  return new Promise((ok, fail) => {
    const child = spawn(EYE, ['serve', '--port', '0'], { env, stdio: ['ignore', 'pipe', 'pipe'] });
    let seen = '';
    const timer = setTimeout(() => fail(new Error(`serve did not print its address in 10 s:\n${seen}`)), 10_000);
    const on = (b: Buffer) => {
      seen += b.toString();
      const m = /board: (http:\/\/127\.0\.0\.1:\d+\/\?t=[0-9a-f]+)/.exec(seen);
      if (m) { clearTimeout(timer); ok({ child, url: m[1]! }); }
    };
    child.stdout!.on('data', on);
    child.stderr!.on('data', on);
    child.on('exit', (code) => { clearTimeout(timer); fail(new Error(`serve exited with ${code}:\n${seen}`)); });
  });
}

export const test = base.extend<{ look: string; eye: Eye; board: Page }>({
  /** The look the board is drawn in (a project option: `chromium-iris` sets `iris`). */
  look: ['ink', { option: true }],

  eye: async ({}, use) => {
    const dir = mkdtempSync(join(tmpdir(), 'eye-e2e-'));
    const bin = join(dir, 'bin');
    mkdirSync(bin);
    for (const n of ['claude', 'codex', 'agy', 'opencode']) {
      const f = join(bin, n);
      writeFileSync(f, FAKE_AGENT);
      chmodSync(f, 0o755);
    }
    const env: NodeJS.ProcessEnv = {
      PATH: `${bin}:/usr/bin:/bin`,
      HOME: join(dir, 'home'),
      EYE_HOME: join(dir, 'eye'),
      EYE_CLAUDE_DIR: join(dir, 'claude'),
      EYE_FIXED_NOW_MS: String(NOW),
      TZ: 'UTC',
      NO_COLOR: '1',
    };
    execFileSync(FIXTURE, [dir], { env });
    const { child, url } = await startServer(dir, env);
    const base_ = new URL(url).origin;
    const token = new URL(url).searchParams.get('t')!;
    const api = async (path: string, body?: unknown) => {
      const r = await fetch(base_ + path, {
        method: body === undefined ? 'GET' : 'POST',
        headers: { 'X-Eye-Token': token, Origin: base_, 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      const text = await r.text();
      return text ? JSON.parse(text) : null;
    };
    await use({
      dir, base: base_, token, url, env, api,
      cli: (...args) => execFileSync(EYE, args, { env, encoding: 'utf8' }),
      addProcess: (pid) => execFileSync(FIXTURE, [dir, '--proc', String(pid)], { env, encoding: 'utf8' }).trim(),
    });
    child.kill('SIGTERM');
    rmSync(dir, { recursive: true, force: true });
  },

  /** A page on the fixture board, in the project's look, with the browser clock pinned to NOW and every console problem recorded in `page.problems`. */
  board: async ({ page, eye, look }, use) => {
    const problems: string[] = [];
    page.on('console', (m) => { if (m.type() === 'error' || m.type() === 'warning') problems.push(`console ${m.type()}: ${m.text()}`); });
    page.on('pageerror', (e) => problems.push(`page error: ${e.message}`));
    page.on('requestfailed', (r) => problems.push(`request failed: ${r.url()} ${r.failure()?.errorText}`));
    page.on('response', (r) => { if (r.status() >= 400) problems.push(`http ${r.status()}: ${r.url()}`); });
    (page as Page & { problems: string[] }).problems = problems;
    // what is in flight, but the event stream, which stays open for good (see `settled`)
    const inflight = new Set<Request>();
    const stream = (r: Request) => new URL(r.url()).pathname === '/api/events';
    page.on('request', (r) => { if (!stream(r)) inflight.add(r); });
    page.on('requestfinished', (r) => inflight.delete(r));
    page.on('requestfailed', (r) => inflight.delete(r));
    (page as Page & { inflight: Set<Request> }).inflight = inflight;
    await page.clock.setFixedTime(NOW);
    await page.goto(look === 'ink' ? eye.url : `${eye.url}&look=${look}`);
    // every test checks it really runs in its look, not in the default
    await expect(page.locator('html')).toHaveAttribute('data-look', look);
    await expect(page.getByRole('region', { name: /^needs you/i })).toBeVisible();
    await use(page);
  },
});

export const problems = (p: Page) => (p as Page & { problems: string[] }).problems;

/**
 * Wait until the page has nothing in flight for half a second: what `networkidle` means, but the
 * event stream, which the page reads with `fetch` and keeps open, does not count (the browser would
 * never call the page idle).
 */
export async function settled(p: Page) {
  const inflight = (p as Page & { inflight: Set<Request> }).inflight;
  await p.waitForLoadState('load');
  for (;;) {
    await expect.poll(() => inflight.size).toBe(0);
    await p.waitForTimeout(500);
    if (inflight.size === 0) return;
  }
}

/** A card on the board by part of its title. */
export const card = (p: Page, title: RegExp | string) => p.getByRole('button', { name: title }).first();

/** A lane by its title. */
export const lane = (p: Page, name: RegExp | string) => p.getByRole('region', { name });

/** The Done rail (Done starts folded; its name says how many it holds). */
export const done = (p: Page) => p.getByRole('button', { name: /^Done, \d+ cards?\. Unfold/ });
