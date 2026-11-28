import { demoClock, emptyClock } from '../../face/src/data/clockSample';
import { hintClock, tooFewClock } from '../../face/src/data/clockStates';
import { spawn } from 'node:child_process';
import { card, done, expect, lane, problems, settled, sid, test } from '../support';

/** Forget what the page keeps as settings, but not its token: that is how it gets in (the link is not opened again). */
const forgetSettings = (p: import('@playwright/test').Page) =>
  p.evaluate(() => { const t = localStorage.getItem('eye.token')!; localStorage.clear(); localStorage.setItem('eye.token', t); });

test.describe('board', () => {
  test('loads with every lane and no console problems', async ({ board }) => {
    for (const [name, n] of [['ideas', 3], ['running', 2], ['needs you', 5], ['waiting', 3], ['paused', 2]] as const) {
      await expect(lane(board, new RegExp(`^${name}`, 'i')).locator('[data-id]')).toHaveCount(n);
    }
    // Done starts folded to a rail that says how many it holds
    await expect(done(board)).toHaveAccessibleName('Done, 4 cards. Unfold');
    await expect(board.getByRole('button', { name: /Tonight: 2/ })).toBeVisible();
    await settled(board);
    expect(problems(board)).toEqual([]);
  });

  test('both agents, a remote session and the long project name show on their cards', async ({ board }) => {
    await expect(lane(board, /^running/i).getByText(/codex/i).first()).toBeVisible();
    // Remote Control is a mark on the card's top edge: its icon, its words in the title
    await expect(lane(board, /^waiting/i).getByTitle(/Remote Control is on/)).toBeVisible();
    await expect(lane(board, /^running/i)).toContainText('customer-billing');
  });
});

test.describe('search', () => {
  const box = (p: import('@playwright/test').Page) => p.getByPlaceholder('Search every session');

  test('words find the session, the best match first', async ({ board }) => {
    await box(board).fill('stale tokens');
    await expect(board.getByRole('heading', { name: /RESULTS?/ })).toBeVisible();
    await expect(board.locator('article[role=button]').first()).toContainText('Fix stale tokens in the account store');
  });

  test('a typo still finds it', async ({ board }) => {
    await box(board).fill('stael tokens');
    await expect(board.locator('article[role=button]').first()).toContainText('Fix stale tokens in the account store');
  });

  test('a pasted link finds the session that opened that pull request', async ({ board }) => {
    await box(board).fill('https://github.com/acme/payments-api/pull/412');
    await expect(board.locator('article[role=button]').first()).toContainText('Move invoices to the new queue');
  });

  test('Escape clears the search and the board comes back', async ({ board }) => {
    await box(board).fill('stale tokens');
    await expect(board.locator('article[role=button]').first()).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(lane(board, /^running/i)).toBeVisible();
  });
});

test.describe('panel', () => {
  test('a card opens its panel with the sections that belong to it', async ({ board }) => {
    await card(board, /Fix stale tokens/).click();
    await expect(board.getByRole('heading', { level: 2, name: 'Fix stale tokens in the account store' })).toBeVisible();
    await expect(board.getByRole('button', { name: 'Close (Esc)' })).toBeVisible();
    for (const t of ['LEFTOVERS', 'REMINDER']) await expect(board.getByRole('heading', { name: new RegExp(t) })).toBeVisible();
    await expect(board.getByRole('button', { name: 'MORE ▾' })).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(board.getByRole('button', { name: 'Close (Esc)' })).toHaveCount(0);
  });

  test('D moves the open session to Done', async ({ board, eye }) => {
    await card(board, /Retry failed webhook/).click();
    await expect(board.getByRole('heading', { level: 2, name: /Retry failed webhook/ })).toBeVisible();
    await board.keyboard.press('d');
    await expect(done(board)).toHaveAccessibleName('Done, 5 cards. Unfold');
    await expect.poll(async () => (await eye.api(`/api/cards/${sid(3)}`)).column).toBe('done');
  });

  test('? lists the keys, T opens CONTINUE TONIGHT and N still moves to Needs you', async ({ board, eye }) => {
    await card(board, /Fix stale tokens/).click();
    await expect(board.getByRole('heading', { level: 2, name: /Fix stale tokens/ })).toBeVisible();
    await board.keyboard.press('?');
    const keys = board.getByRole('dialog', { name: 'Keyboard shortcuts' });
    await expect(keys).toBeVisible();
    for (const k of ['N', 'T', 'X', 'R']) await expect(keys.getByText(k, { exact: true })).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(keys).toHaveCount(0);
    await board.keyboard.press('t');
    await expect(board.getByRole('form', { name: 'Continue tonight' })).toBeVisible();
    await board.getByRole('form', { name: 'Continue tonight' }).getByRole('textbox').first().blur();
    await board.keyboard.press('n');
    await expect.poll(async () => (await eye.api(`/api/cards/${sid(4)}`)).column).toBe('needs_you');
  });
});

test.describe('menu', () => {
  test('right-click lists the actions of a session and Mark done moves it', async ({ board }) => {
    await card(board, /Fix the flaky checkout/).click({ button: 'right' });
    const menu = board.getByRole('menu', { name: /Actions for Fix the flaky checkout/ });
    for (const t of ['MARK DONE', 'OPEN DETAILS', 'HIDE (USELESS)', 'NEEDS YOU', 'WAITING', 'TOMORROW 9:00', 'PICK A DATE…'])
      await expect(menu.getByRole('menuitem', { name: t })).toBeVisible();
    await menu.getByRole('menuitem', { name: 'MARK DONE' }).click();
    await expect(done(board)).toHaveAccessibleName('Done, 5 cards. Unfold');
    await done(board).click();
    await expect(lane(board, /^done/i).getByRole('button', { name: /Fix the flaky checkout/ })).toBeVisible();
  });

  test('an idea card has its own menu', async ({ board }) => {
    await card(board, /Write a changelog generator/).click({ button: 'right' });
    const menu = board.getByRole('menu', { name: /Actions for Write a changelog/ });
    for (const t of ['EDIT', 'OPEN DETAILS']) await expect(menu.getByRole('menuitem', { name: t })).toBeVisible();
  });

  test('CONTINUE TONIGHT… in a session menu opens the form with its prompt focused', async ({ board }) => {
    // a laptop-height window: the panel is longer than the screen
    await board.setViewportSize({ width: 1280, height: 600 });
    await card(board, /Fix stale tokens/).click({ button: 'right' });
    const menu = board.getByRole('menu', { name: /Actions for Fix stale tokens/ });
    await menu.getByRole('menuitem', { name: 'CONTINUE TONIGHT…' }).click();
    const form = board.getByRole('form', { name: 'Continue tonight' });
    await expect(form).toBeVisible();
    await expect(form.getByRole('textbox').first()).toBeFocused();
    // in view, also after the panel's other sections have loaded above it
    await board.waitForTimeout(800);
    await expect(form.getByRole('textbox').first()).toBeInViewport();
  });
});

test.describe('reminders', () => {
  test('Remind me shows on the card and in the panel', async ({ board, eye }) => {
    await card(board, /Fix the flaky checkout/).click({ button: 'right' });
    await board.getByRole('menuitem', { name: 'TOMORROW 9:00' }).click();
    await expect.poll(async () => (await eye.api(`/api/cards/${sid(8)}`)).remind_at).toBe(Date.UTC(2026, 10, 21, 9));
    await expect(card(board, /Fix the flaky checkout/).getByRole('img', { name: /remind/i })).toBeVisible();
    await card(board, /Fix the flaky checkout/).click();
    // folded with the details: what is set in its heading, the rest when opened
    const panel = board.locator('#reminder');
    await expect(panel).toContainText(/Sat.*9:00|9:00/);
    await panel.getByRole('button', { name: /^REMINDER/ }).click();
    await expect(panel).not.toContainText('No reminder.');
    await panel.getByRole('button', { name: 'CLEAR' }).click();
    await expect(panel).toContainText('No reminder.');
    expect((await eye.api(`/api/cards/${sid(8)}`)).remind_at).toBeNull();
  });
});

test.describe('ideas', () => {
  test('the idea form makes an idea and plans it for tonight', async ({ board, eye }) => {
    await board.getByRole('button', { name: '+ IDEA' }).click();
    const form = board.getByRole('form', { name: 'New idea' });
    await form.getByLabel('TITLE').fill('Try the new linter');
    await form.getByLabel(/^PROMPT FOR/).fill('Run the linter on the whole repo and list what it finds.');
    await form.getByRole('textbox', { name: 'FOLDER' }).fill('~/code/notes-app');
    await form.getByRole('button', { name: 'Run it tonight' }).click();
    await form.getByRole('button', { name: 'SAVE IDEA' }).click();
    await expect(form).toHaveCount(0);
    await expect(lane(board, /^ideas/i).getByRole('button', { name: /Try the new linter/ })).toBeVisible();
    const night = await eye.api('/api/night');
    expect(night.enabled).toBe(false);
    expect(night.planned).toBe(3);
    expect(night.open.map((r: any) => r.prompt)).toContain('Run the linter on the whole repo and list what it finds.');
    await expect(board.getByRole('button', { name: /Tonight: 3/ })).toBeVisible();
  });

  test("an idea's panel has one action row and takes its keys", async ({ board }) => {
    await card(board, /Write a changelog generator/).click();
    const page = board.locator('.page');
    await expect(page.getByRole('heading', { level: 2, name: /Write a changelog generator/ })).toBeVisible();
    // START, IN BACKGROUND, TONIGHT and EDIT on one line, ARCHIVE in MORE
    const mids = await page.locator(':scope > .acts .btn').evaluateAll((bs) => bs.map((b) => { const r = b.getBoundingClientRect(); return Math.round(r.top + r.height / 2); }));
    expect(mids.length).toBe(4);
    expect(new Set(mids).size).toBe(1);
    await page.getByRole('button', { name: 'MORE ▾' }).click();
    await expect(page.getByRole('menuitem', { name: /ARCHIVE/ })).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(page.getByRole('menuitem', { name: /ARCHIVE/ })).toHaveCount(0);
    await board.keyboard.press('?');
    const keys = board.getByRole('dialog', { name: 'Keyboard shortcuts' });
    await expect(keys).toContainText('Start in the background');
    await board.keyboard.press('Escape');
    await expect(keys).toHaveCount(0);
    await board.keyboard.press('e');
    await expect(board.getByRole('form', { name: 'Edit idea' })).toBeVisible();
  });

  test('a title is required', async ({ board }) => {
    await board.getByRole('button', { name: '+ IDEA' }).click();
    const form = board.getByRole('form', { name: 'New idea' });
    await form.getByRole('button', { name: 'SAVE IDEA' }).click();
    await expect(form).toBeVisible();
  });
});

test.describe('usage', () => {
  test('the panel has the totals, the rows and the plan limits', async ({ board }) => {
    await board.getByRole('button', { name: 'USAGE' }).click();
    const panel = board.getByRole('complementary', { name: 'Usage' });
    await expect(panel.getByRole('heading', { name: /THIS WEEK BY PROJECT/ })).toBeVisible();
    for (const t of ['BY AGENT', 'TOP 10 SESSIONS', 'LAST 4 WEEKS', 'PLAN LIMITS']) await expect(panel.getByRole('heading', { name: new RegExp(t) })).toBeVisible();
    await expect(panel).toContainText('$');
    await expect(panel).toContainText('payments-api');
    await panel.getByRole('button', { name: /month/i }).click();
    await expect(panel.getByRole('heading', { name: /THIS MONTH BY PROJECT/ })).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
  });
});

// The clock route is C2's; these flows answer it from the face's synthetic data so every state is drawn.
const answer = (board: import('@playwright/test').Page, body: unknown, status = 200) =>
  board.route('**/api/clock*', (r) => r.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) }));

test.describe('usage chart', () => {
  test('draws the daily cost, splits by model and has a table twin', async ({ board }) => {
    await board.getByRole('button', { name: 'USAGE' }).click();
    const panel = board.getByRole('complementary', { name: 'Usage' });
    await expect(panel.getByRole('heading', { name: /DAILY COST/ })).toBeVisible();
    await panel.getByRole('button', { name: 'BY MODEL' }).click();
    await expect(panel.getByRole('button', { name: 'BY MODEL' })).toHaveAttribute('aria-pressed', 'true');
    await panel.getByRole('button', { name: 'TABLE' }).first().click();
    await expect(panel.locator('.ctable').first()).toBeVisible();
  });
});

// It is evening and hints are on: the engine says so in the words it uses.
const evening = () => {
  const c = hintClock();
  return { ...c, now: { ...c.now, hint: { text: 'Evening: your corrections usually run 1.8x now.', offer: true } } };
};

test.describe('hours', () => {
  test('data: the verdict, the diagram and a tooltip per hour', async ({ board }) => {
    await answer(board, demoClock(4));
    await board.getByRole('button', { name: 'HOURS', exact: true }).click();
    const panel = board.getByRole('complementary', { name: 'Hours' });
    await expect(panel.locator('.verdict')).toContainText('Mostly you.');
    await expect(panel.locator('.cdiagram')).toBeVisible();
    await panel.locator('.cfocus').focus();
    await board.keyboard.press('Home');
    await expect(panel.locator('.ctip')).toContainText('00:00–01:00');
    await panel.getByRole('button', { name: 'TABLE' }).first().click();
    await expect(panel.locator('.ctable tbody tr').first()).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
  });

  test('too few: a plain sentence and no chart', async ({ board }) => {
    await answer(board, tooFewClock());
    await board.getByRole('button', { name: 'HOURS', exact: true }).click();
    const panel = board.getByRole('complementary', { name: 'Hours' });
    await expect(panel).toContainText('Not enough data yet: 120 prompts, 6 corrections.');
    await expect(panel.locator('.cdiagram')).toHaveCount(0);
  });

  test('empty: says nothing is recorded; a missing route says not available', async ({ board }) => {
    await answer(board, emptyClock(4));
    await board.getByRole('button', { name: 'HOURS', exact: true }).click();
    await expect(board.getByRole('complementary', { name: 'Hours' })).toContainText('Nothing recorded yet');
    await board.keyboard.press('Escape');
    await board.unroute('**/api/clock*');
    await answer(board, { error: 'no' }, 404);
    await board.getByRole('button', { name: 'HOURS', exact: true }).click();
    await expect(board.getByRole('complementary', { name: 'Hours' })).toContainText('Not available in this version');
  });
});

test.describe('settings', () => {
  test('the gear changes the theme and the layout, keeps the shortcuts, and closes on Esc', async ({ board, eye, look }) => {
    const panel = board.getByRole('complementary', { name: 'Settings' });
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    await expect(panel.getByRole('heading', { name: 'LOOK' })).toBeVisible();
    await panel.getByRole('group', { name: 'Theme' }).getByRole('button', { name: 'DARK' }).click();
    await expect(board.locator('html')).toHaveAttribute('data-theme', 'dark');
    await panel.getByRole('group', { name: 'Layout' }).getByRole('button', { name: 'GRID' }).click();
    await expect(board.locator('.board.gridview')).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    // the board's keys still work: `,` brings the panel back, Esc closes it, `/` goes to the search
    await board.keyboard.press(',');
    await expect(panel).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    await board.keyboard.press('/');
    await expect(board.getByPlaceholder('Search every session')).toBeFocused();
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    await panel.getByRole('group', { name: 'Layout' }).getByRole('button', { name: 'LANES' }).click();
    await expect(board.locator('.board.lanes')).toBeVisible();
    // the choices are the engine's too, under the look's own keys
    const prefs = await eye.api('/api/prefs');
    expect(prefs['eye.theme']).toBe('dark');
    expect(prefs[`eye.${look}.layout`]).toBe('lanes');
  });

  test('an agent setting is saved into the config at once, and a key left to the file is refused', async ({ board, eye }) => {
    const panel = board.getByRole('complementary', { name: 'Settings' });
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    await panel.getByRole('button', { name: /^SUMMARIES/ }).click();
    await panel.getByRole('group', { name: 'Summaries' }).getByRole('button', { name: 'ON' }).click();
    await expect(panel.getByRole('status')).toHaveText('Saved');
    expect((await eye.api('/api/settings')).settings['summary.enabled'].value).toBe(true);
    await expect(panel).toContainText('config.toml');
    expect((await eye.api('/api/settings', { 'board.port': 1 })).error).toMatch(/not a setting/);
  });

  test('only LOOK is open, each folded heading says what it holds, and a heading opens its section', async ({ board }) => {
    const panel = board.getByRole('complementary', { name: 'Settings' });
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    const heads = panel.locator('h4 > button');
    await expect(heads).toHaveCount(7);
    const read = () => heads.evaluateAll((bs) => bs.map((b) => [b.firstChild?.textContent, b.getAttribute('aria-expanded'), b.querySelector('.fsum')?.textContent ?? null]));
    expect(await read()).toEqual([
      ['LOOK', 'true', null],
      ['SUMMARIES', 'false', 'OFF'],
      ['STARTED BY PROGRAMS', 'false', '4 FOUND · OFF'],
      ['NIGHT SHIFT', 'false', 'OFF'],
      ['REMINDERS', 'false', 'OFF'],
      ['HOURS', 'false', 'ON'],
      ['AGENTS', 'false', 'Claude Code, Codex CLI, Antigravity CLI, OpenCode'],
    ]);
    await expect(panel.getByRole('heading', { name: /BOARD/ })).toHaveCount(0);
    // the keyboard reaches a heading, Enter opens it, Esc closes the panel
    await panel.getByRole('button', { name: /^NIGHT SHIFT/ }).focus();
    await board.keyboard.press('Enter');
    await expect(panel.getByRole('button', { name: /^NIGHT SHIFT/ })).toHaveAttribute('aria-expanded', 'true');
    await expect(panel.getByRole('group', { name: 'Night shift' })).toBeVisible();
    await board.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
  });

  test('the agents and the steering clock are shown with how to change them, and the API still takes both', async ({ board, eye }) => {
    const panel = board.getByRole('complementary', { name: 'Settings' });
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    await panel.getByRole('button', { name: /^HOURS/ }).click();
    await panel.getByRole('button', { name: /^AGENTS/ }).click();
    await expect(panel.getByRole('group', { name: 'Steering clock' })).toHaveCount(0);
    await expect(panel.getByRole('group', { name: 'Agents' })).toHaveCount(0);
    await expect(panel.locator('.srow', { hasText: 'STEERING CLOCK' }).locator('.sval')).toHaveText('ON');
    await expect(panel).toContainText('To change it, set [clock] enabled in ~/.agent-eye/config.toml, then run eye restart.');
    await expect(panel.locator('.srow', { hasText: 'FOLLOWED' }).locator('.sval')).toHaveText('Claude Code, Codex CLI, Antigravity CLI, OpenCode');
    await expect(panel).toContainText('To change it, set [agents] enabled in ~/.agent-eye/config.toml, then run eye restart.');
    // the rest of HOURS still saves at once
    await panel.getByRole('group', { name: 'Hint' }).getByRole('button', { name: 'ON' }).click();
    await expect(panel.getByRole('status')).toHaveText('Saved');
    // no breaking change: the keys are still the API's
    const s = (await eye.api('/api/settings', { 'clock.enabled': false, 'agents.enabled': ['claude'] })).settings;
    expect([s['clock.enabled'].value, s['agents.enabled'].value, s['clock.hints'].value]).toEqual([false, ['claude'], true]);
  });

  test('the model is a choice of the summary agent\'s names, and Other… opens a field for any other', async ({ board, eye }) => {
    const panel = board.getByRole('complementary', { name: 'Settings' });
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    await panel.getByRole('button', { name: /^SUMMARIES/ }).click();
    const model = panel.getByRole('combobox', { name: 'MODEL FOR SUMMARIES' });
    await expect(model.locator('option')).toHaveText(['Default: haiku', 'sonnet', 'opus', 'Other…']);
    await expect(panel).toContainText('The model Claude Code writes summaries with, and judges corrections with from the next nightly pass.');
    await model.selectOption('sonnet');
    await expect(panel.getByRole('status')).toHaveText('Saved');
    expect((await eye.api('/api/settings')).settings['summary.model'].value).toBe('sonnet');
    await expect(panel.getByRole('textbox', { name: 'MODEL NAME' })).toHaveCount(0);
    await model.selectOption({ label: 'Other…' });
    const field = panel.getByRole('textbox', { name: 'MODEL NAME' });
    await expect(field).toBeFocused();
    await field.fill('claude-small-2');
    await panel.getByRole('button', { name: 'SET', exact: true }).click();
    await expect(field).toHaveCount(0);
    await expect(model).toHaveValue('claude-small-2');
    expect((await eye.api('/api/settings')).settings['summary.model'].value).toBe('claude-small-2');
    await model.selectOption('');
    await expect.poll(async () => (await eye.api('/api/settings')).settings['summary.model'].value).toBe('');
  });

  test('quiet App, switched by keyboard, takes the app session out of Needs you at once and keeps it listed', async ({ board, eye }) => {
    const needsYou = lane(board, /^needs you/i);
    const worst = /Rework the customer billing reconciliation/;
    await expect(needsYou.locator('[data-id]')).toHaveCount(5);
    await expect(needsYou.getByRole('button', { name: worst })).toBeVisible();
    const panel = board.getByRole('complementary', { name: 'Settings' });
    await board.keyboard.press(',');
    const head = panel.getByRole('button', { name: /^STARTED BY PROGRAMS/ });
    await head.focus();
    await board.keyboard.press('Enter');
    await expect(head).toHaveAttribute('aria-expanded', 'true');
    await expect(panel).toContainText("2 sessions started by an app through an agent's SDK; quiet would take 1 out of Needs you.");
    await expect(panel).toContainText('2 sessions started by a script or a CI job; quiet would take none out of Needs you.');
    const on = panel.getByRole('group', { name: 'Quiet App' }).getByRole('button', { name: 'ON' });
    await board.keyboard.press('Tab');
    await expect(on).toBeFocused();
    await board.keyboard.press('Enter');
    await expect(panel.getByRole('status')).toHaveText('Saved');
    await expect(on).toHaveAttribute('aria-pressed', 'true');
    await expect(panel).toContainText("2 sessions started by an app through an agent's SDK; 1 kept out of Needs you.");
    expect((await eye.api('/api/settings')).settings['board.quiet'].value).toEqual(['app']);
    await board.keyboard.press('Escape');
    // the board follows without a restart: a program's run is in the archive, out of the lanes, and still found
    await expect(needsYou.locator('[data-id]')).toHaveCount(4);
    await expect(board.locator('main').getByRole('button', { name: worst })).toHaveCount(0);
    await board.getByPlaceholder('Search every session').fill('reconciliation export');
    await expect(board.locator('article[role=button]').first()).toContainText('Rework the customer billing reconciliation');
    // and off again, it is back
    await eye.api('/api/settings', { 'board.quiet': [] });
    await board.reload();
    await expect(needsYou.getByRole('button', { name: worst })).toBeVisible();
  });

  test('on a phone, with every section open and Other… showing its field, nothing scrolls sideways', async ({ board }) => {
    await board.setViewportSize({ width: 360, height: 800 });
    const panel = board.getByRole('complementary', { name: 'Settings' });
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    for (const t of ['SUMMARIES', 'STARTED BY PROGRAMS', 'NIGHT SHIFT', 'REMINDERS', 'HOURS', 'AGENTS']) await panel.getByRole('button', { name: new RegExp(`^${t}`) }).click();
    await panel.getByRole('combobox', { name: 'MODEL FOR SUMMARIES' }).selectOption({ label: 'Other…' });
    await expect(panel.getByRole('textbox', { name: 'MODEL NAME' })).toBeVisible();
    const wide = await board.evaluate(() => {
      const over = (el: Element) => el.scrollWidth > el.clientWidth + 1;
      return [document.documentElement, ...document.querySelectorAll('.settings, .settings *')].filter(over).map((el) => `${el.tagName}.${el.className}`);
    });
    expect(wide).toEqual([]);
    for (const b of await panel.locator('h4 > button').all()) {
      const box = (await b.boundingBox())!;
      expect(box.x + box.width).toBeLessThanOrEqual(360);
    }
  });
});

test.describe('leftovers', () => {
  test('Ignore asks first, and a yes removes the item', async ({ board, eye }) => {
    await card(board, /Fix stale tokens/).click();
    // the list is folded under the two-line summary
    await board.locator('#leftovers').getByRole('button', { name: 'SHOW' }).click();
    const items = board.locator('ul.dloose > li');
    await expect(items).toHaveCount(3);
    const git = items.filter({ hasText: '2 commits in auth-service' });
    await git.getByRole('button', { name: 'IGNORE' }).click();
    const ask = board.getByRole('alertdialog', { name: /Stop listing/ });
    await expect(ask).toBeVisible();
    await ask.getByRole('button', { name: 'CANCEL' }).click();
    await expect(items).toHaveCount(3);
    await git.getByRole('button', { name: 'IGNORE' }).click();
    await board.getByRole('alertdialog', { name: /Stop listing/ }).getByRole('button', { name: 'YES' }).click();
    await expect(items).toHaveCount(2);
    const loose = await eye.api(`/api/loose?session=${sid(4)}`);
    expect(loose.items).toHaveLength(2);
    expect(loose.ignored).toBe(1);
  });

  test('Stop asks first and ends a process the test started', async ({ board, eye }) => {
    const child = spawn('sleep', ['600'], { stdio: 'ignore' });
    const gone = new Promise<void>((ok) => child.on('exit', () => ok()));
    try {
      expect(eye.addProcess(child.pid!)).toMatch(/^proc:\d+:\d+$/);
      await board.reload();
      await card(board, /Fix stale tokens/).click();
      await board.locator('#leftovers').getByRole('button', { name: 'SHOW' }).click();
      const item = board.locator('ul.dloose > li').filter({ hasText: `sleep (pid ${child.pid})` });
      await item.getByRole('button', { name: 'STOP' }).click();
      const ask = board.getByRole('alertdialog', { name: `Stop sleep (pid ${child.pid})?` });
      await expect(ask).toBeVisible();
      expect(child.exitCode).toBeNull();
      await ask.getByRole('button', { name: 'YES' }).click();
      await gone;
      await expect(item).toHaveCount(0);
    } finally {
      child.kill('SIGKILL');
    }
  });
});

test.describe('night', () => {
  test('the Tonight chip opens the list with the planned and the finished runs', async ({ board, eye }) => {
    await board.getByRole('button', { name: /Tonight: 2/ }).click();
    const panel = board.getByRole('complementary', { name: 'Tonight' });
    await expect(panel.getByRole('heading', { name: /PLANNED AND RUNNING/ })).toBeVisible();
    await expect(panel.getByRole('button', { name: 'NIGHT SHIFT OFF' })).toBeVisible();
    for (const t of ['Planned', 'Running', 'Done', 'Failed', 'Skipped']) await expect(panel.getByText(new RegExp(`^${t}`)).first()).toBeVisible();
    // cancelling a planned run removes it; nothing ever starts because night is off
    await panel.getByRole('button', { name: 'CANCEL RUN' }).first().click();
    await expect.poll(async () => (await eye.api('/api/night')).planned).toBe(1);
    expect((await eye.api('/api/night')).enabled).toBe(false);
  });
});

test.describe('access', () => {
  test('the link\'s token is kept by the page, taken out of the address and never put in a cookie', async ({ board, eye, context }) => {
    expect(new URL(board.url()).searchParams.has('t')).toBe(false);
    expect(await board.evaluate(() => localStorage.getItem('eye.token'))).toBe(eye.token);
    expect(await context.cookies()).toEqual([]);
    // every call of the page carries the token in a header, and no address does
    const calls: Array<{ url: string; token: string | undefined; cookie: string | undefined }> = [];
    board.on('request', (r) => {
      if (new URL(r.url()).pathname.startsWith('/api/')) calls.push({ url: r.url(), token: r.headers()['x-eye-token'], cookie: r.headers().cookie });
    });
    await board.reload();
    await expect(board.getByRole('region', { name: /^needs you/i })).toBeVisible();
    expect(calls.length).toBeGreaterThan(3);
    for (const c of calls) {
      expect(c.token, c.url).toBe(eye.token);
      expect(c.cookie, c.url).toBeUndefined();
      expect(c.url).not.toContain(eye.token);
    }
    expect(await context.cookies()).toEqual([]);
  });

  test('a page opened again without the link still has its token, as the installed app does after a restart', async ({ board, eye }) => {
    await board.goto(eye.base + '/');
    await expect(board.getByRole('region', { name: /^needs you/i })).toBeVisible();
    await expect(board.locator('.err')).toHaveCount(0);
  });

  test('a page that never had the link says how to open the board and asks the API for nothing', async ({ page, eye }) => {
    const asked: string[] = [];
    page.on('request', (r) => { if (new URL(r.url()).pathname.startsWith('/api/')) asked.push(r.url()); });
    await page.goto(eye.base + '/');
    await expect(page.getByText(/run `eye board` in a terminal and open the link it prints/)).toBeVisible();
    expect(asked).toEqual([]);
  });

  test('a token the engine refuses is dropped, and the page says the same words', async ({ page, eye }) => {
    await page.goto(`${eye.base}/?t=${'f'.repeat(64)}`);
    await expect(page.getByText(/run `eye board` in a terminal and open the link it prints/)).toBeVisible();
    expect(new URL(page.url()).searchParams.has('t')).toBe(false);
    expect(await page.evaluate(() => localStorage.getItem('eye.token'))).toBeNull();
  });

  test('a cookie from an older eye opens nothing, and the next visit takes it away', async ({ page, context, eye }) => {
    await context.addCookies([{ name: 'eye_token', value: eye.token, url: eye.base }]);
    expect((await context.request.get(`${eye.base}/api/board`)).status()).toBe(401);
    await page.goto(`${eye.base}/`);
    await expect.poll(async () => (await context.cookies()).filter((c) => c.name === 'eye_token')).toEqual([]);
  });

  test('the event stream works with the header: a change made elsewhere shows without a reload', async ({ board, eye }) => {
    await expect(board.locator('.conn')).toHaveText(/LIVE/);
    const stale = (await eye.api('/api/search?q=stale%20tokens')).results[0].card.id;
    await expect(card(board, /Fix stale tokens/)).toBeVisible();
    // the engine looks at its database every 2 s and tells the page about a change it sees after its first look
    await board.waitForTimeout(2500);
    await eye.api(`/api/cards/${stale}/hidden`, { hidden: true });
    await expect(card(board, /Fix stale tokens/)).toHaveCount(0);
  });

  test('the app manifest is linked with the token in its address, and bare it is refused', async ({ board, eye }) => {
    const href = await board.locator('link[rel=manifest]').getAttribute('href');
    expect(href).toBe(`/manifest.webmanifest?t=${eye.token}`);
    const m = await board.request.get(eye.base + href!);
    expect(m.ok()).toBe(true);
    expect((await m.json()).start_url).toBe(`/?t=${eye.token}`);
    expect((await board.request.get(`${eye.base}/manifest.webmanifest`)).status()).toBe(401);
  });
});

test.describe('export', () => {
  test('Export downloads one self-contained HTML page with the secret masked', async ({ board }) => {
    await card(board, /Fix stale tokens/).click();
    await board.getByRole('button', { name: 'MORE ▾' }).click();
    const [download] = await Promise.all([board.waitForEvent('download'), board.getByRole('menuitem', { name: /^EXPORT/ }).click()]);
    expect(download.suggestedFilename()).toMatch(/^eye-e2e00000.*\.html$/);
    const path = await download.path();
    const html = (await import('node:fs')).readFileSync(path, 'utf8');
    expect(html).toContain('Fix stale tokens in the account store');
    expect(html).toContain('expire them softly');
    expect(html).not.toContain('ghp_AAAAAAAA');
  });
});

test.describe('looks', () => {
  test('the settings panel switches the look, and the engine remembers it for the next page', async ({ board, eye, look }) => {
    const other = look === 'ink' ? 'iris' : 'ink';
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    const looks = board.getByRole('group', { name: 'Look' });
    await expect(looks.getByRole('button', { name: look.toUpperCase() })).toHaveAttribute('aria-pressed', 'true');
    await looks.getByRole('button', { name: other.toUpperCase() }).click();
    await expect(board.locator('html')).toHaveAttribute('data-look', other);
    await expect(board.getByRole('region', { name: /^needs you/i })).toBeVisible();
    expect((await eye.api('/api/prefs'))['eye.look']).toBe(other);
    // a new page without ?look= opens in the saved look: the engine's, with this browser's copy gone
    await forgetSettings(board);
    await board.goto(eye.base + '/');
    await expect(board.locator('html')).toHaveAttribute('data-look', other);
    await expect(board.getByRole('region', { name: /^needs you/i })).toBeVisible();
    // leaving a page cancels its event stream (WebKit reports that); anything else is a problem
    expect(problems(board).filter((p) => !/\/api\/events .*(cancelled|ERR_ABORTED)/i.test(p))).toEqual([]);
  });

  test('iris: each lane head says one fact and draws one tick per session', async ({ board, look }) => {
    test.skip(look !== 'iris', 'the lane facts and timelines are iris parts');
    const head = (name: RegExp) => lane(board, name).locator('.chead');
    await expect(head(/^needs you/i).locator('.lfact')).toHaveText('oldest ask 1d');
    await expect(head(/^waiting/i).locator('.lfact')).toHaveText('on Dana');
    await expect(head(/^running/i).locator('.lfact')).toHaveText('2 working now');
    await expect(head(/^paused/i).locator('.lfact')).toHaveText('2 this week');
    for (const [name, n] of [['ideas', 3], ['running', 2], ['needs you', 5], ['waiting', 3], ['paused', 2]] as const) {
      await expect(head(new RegExp(`^${name}`, 'i')).locator('.tline .tk')).toHaveCount(n);
    }
    // the title block under the board counts what is open and what is done
    await expect(board.locator('.tblock')).toContainText('15 open · 4 done · 0 archived');
    // no stamps per session in iris: a card is told apart by its words
    await expect(board.locator('.sigil')).toHaveCount(0);
  });

  test('each look keeps its own layout: Grid in one, Lanes in the other, both back after switching', async ({ board, eye, look }) => {
    const other = look === 'ink' ? 'iris' : 'ink';
    const pick = async (name: string) => {
      await board.getByRole('button', { name: 'Settings', exact: true }).click();
      await board.getByRole('group', { name: 'Look' }).getByRole('button', { name: name.toUpperCase() }).click();
      await expect(board.locator('html')).toHaveAttribute('data-look', name);
    };
    await board.getByRole('button', { name: 'Settings', exact: true }).click();
    await board.getByRole('group', { name: 'Layout' }).getByRole('button', { name: 'Grid' }).click();
    await expect(board.locator('.board.gridview')).toBeVisible();
    await board.keyboard.press('Escape');
    await pick(other);
    await expect(board.locator('.board.lanes')).toBeVisible();
    // the layout comes back from the engine, not only from this browser
    await forgetSettings(board);
    await pick(look);
    await expect(board.locator('.board.gridview')).toBeVisible();
    const prefs = await eye.api('/api/prefs');
    expect(prefs[`eye.${look}.layout`]).toBe('grid');
    expect(prefs[`eye.${other}.layout`]).toBeUndefined();
  });

  test('a look in the address holds for that page only; an unknown one falls back to the saved look', async ({ board, eye, look }) => {
    // the board was opened with ?look=<look> (iris) or without it (ink): nothing was saved either way
    expect(await board.evaluate(() => localStorage.getItem('eye.look'))).toBeNull();
    expect((await eye.api('/api/prefs'))['eye.look']).toBeUndefined();
    await eye.api('/api/prefs', { 'eye.look': look });
    await board.goto(`${eye.url}&look=nope`);
    await expect(board.locator('html')).toHaveAttribute('data-look', look);
    await board.goto(`${eye.url}&look=constructor`);
    await expect(board.locator('html')).toHaveAttribute('data-look', look);
    await expect(board.getByRole('region', { name: /^needs you/i })).toBeVisible();
  });

  test('hint: the header mark opens HOURS, the card you work in has its mark, and its menu offers Continue tonight first', async ({ board }) => {
    await answer(board, evening());
    await board.reload();
    const mark = board.getByRole('button', { name: /^Evening: your corrections usually run 1\.8x now/ });
    await expect(mark).toBeVisible();
    await mark.click();
    await expect(board.getByRole('complementary', { name: 'Hours' })).toBeVisible();
    await board.keyboard.press('Escape');
    // only the card you have open carries the mark
    await expect(board.getByRole('img', { name: /^Evening: your corrections/ })).toHaveCount(0);
    await card(board, /Fix stale tokens/).click();
    await expect(board.getByRole('heading', { level: 2, name: /Fix stale tokens/ })).toBeVisible();
    await expect(board.getByRole('img', { name: /^Evening: your corrections/ })).toHaveCount(1);
    await board.keyboard.press('Escape');
    await card(board, /Fix stale tokens/).click({ button: 'right' });
    const items = board.getByRole('menu').getByRole('menuitem');
    await expect(items.first()).toHaveText('CONTINUE TONIGHT…');
    await expect(board.getByRole('menu')).toContainText('Evening: your corrections usually run 1.8x now.');
    await expect(board.getByRole('menu')).not.toContainText('Hand this to the night shift');
    // a card that is not waiting on you gets the usual menu
    await board.keyboard.press('Escape');
    await card(board, /Add dark mode/).click({ button: 'right' });
    await expect(board.getByRole('menu').getByRole('menuitem').first()).toHaveText('MARK DONE');
  });
});

// A chart's words are drawn in SVG user units and the chart is scaled to its box, so the size that counts is the font size
// times the scale on screen. Nothing may be read at under 12 px, in either look, from a phone to a wide screen.
const onScreenSizes = (p: import('@playwright/test').Page, selector: string) =>
  p.locator(selector).evaluateAll((els) => els.map((el) => Number.parseFloat(getComputedStyle(el).fontSize) * (el as SVGTextElement).getScreenCTM()!.a));

for (const width of [360, 1000, 1440]) {
  test.describe(`readable size at ${width}`, () => {
    test.use({ viewport: { width, height: 900 } });

    test('every word in the HOURS chart is at least 12 px on screen', async ({ board }) => {
      await answer(board, demoClock(4));
      await board.getByRole('button', { name: 'HOURS', exact: true }).click();
      await expect(board.locator('.hours .cdiagram')).toBeVisible();
      const sizes = await onScreenSizes(board, '.hours .cdiagram text');
      expect(sizes.length).toBeGreaterThan(10);
      expect(Math.min(...sizes)).toBeGreaterThanOrEqual(12);
    });

    test('every word in the USAGE chart is at least 12 px on screen', async ({ board }) => {
      await board.getByRole('button', { name: 'USAGE' }).click();
      const panel = board.getByRole('complementary', { name: 'Usage' });
      await expect(panel.getByRole('heading', { name: /DAILY COST/ })).toBeVisible();
      const sizes = await onScreenSizes(board, 'aside[aria-label=Usage] svg text');
      expect(sizes.length).toBeGreaterThan(3);
      expect(Math.min(...sizes)).toBeGreaterThanOrEqual(12);
    });
  });
}

// The longest hint the engine words (a break's, at the biggest ratio) must not push the gear or + IDEA out of the header.
test.describe('the header with the longest hint', () => {
  for (const width of [1000, 1280, 1440, 1600]) {
    test(`keeps every header button in view at ${width}`, async ({ board }) => {
      await board.setViewportSize({ width, height: 900 });
      const c = hintClock();
      const text = 'An hour or more since a break: your corrections usually run 9.9x now.';
      await answer(board, { ...c, now: { ...c.now, hint: { text, offer: true } } });
      await board.reload();
      await expect(board.locator('.hourhint')).toBeVisible();
      await expect(board.getByRole('button', { name: 'Settings', exact: true })).toBeInViewport({ ratio: 1 });
      await expect(board.getByRole('button', { name: '+ IDEA' })).toBeInViewport({ ratio: 1 });
      const out = await board.locator('.hd button').evaluateAll((els) => els.flatMap((el) => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && (r.left < 0 || r.right > window.innerWidth) ? [`${el.textContent || el.getAttribute('aria-label')} ${Math.round(r.left)}-${Math.round(r.right)}`] : [];
      }));
      expect(out).toEqual([]);
    });
  }
});

test.describe('on a touch screen', () => {
  test.use({ hasTouch: true });

  test('tapping an HOURS column shows that hour', async ({ board }) => {
    await answer(board, demoClock(4));
    await board.getByRole('button', { name: 'HOURS', exact: true }).click();
    const panel = board.getByRole('complementary', { name: 'Hours' });
    await expect(panel.locator('.cdiagram')).toBeVisible();
    await expect(panel.locator('.ctip')).toHaveCount(0);
    // the middle of hour 12's column: the chart is 720 units wide, its plot starts at 48 and ends 8 short of the edge
    const tapHour12 = async () => {
      const svg = (await panel.locator('.cdiagram').boundingBox())!;
      const x = 48 + ((720 - 48 - 8) / 24) * 12.5;
      await board.touchscreen.tap(svg.x + (svg.width * x) / 720, svg.y + svg.height * 0.8);
      await expect(panel.locator('.ctip')).toContainText('12:00–13:00', { timeout: 1000 });
    };
    // the board's cards still settle into place under the panel for a moment: tap again until the chart holds still
    await expect(tapHour12).toPass();
  });
});
