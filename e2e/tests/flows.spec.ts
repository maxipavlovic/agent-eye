import { spawn } from 'node:child_process';
import { card, done, expect, lane, problems, sid, test } from '../support';

test.describe('board', () => {
  test('loads with every lane and no console problems', async ({ board }) => {
    for (const [name, n] of [['ideas', 3], ['running', 2], ['needs you', 5], ['waiting', 3], ['paused', 2]] as const) {
      await expect(lane(board, new RegExp(`^${name}`, 'i')).locator('[data-id]')).toHaveCount(n);
    }
    // Done starts folded to a rail that says how many it holds
    await expect(done(board)).toHaveAccessibleName('Done, 4 cards. Unfold');
    await expect(board.getByRole('button', { name: /Tonight: 2/ })).toBeVisible();
    await board.waitForLoadState('networkidle');
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
  test('the picker switches the look, and the engine remembers it for the next page', async ({ board, eye, look }) => {
    const other = look === 'ink' ? 'iris' : 'ink';
    await board.getByRole('button', { name: /^Accent colour/ }).click();
    const looks = board.getByRole('group', { name: 'Look' });
    await expect(looks.getByRole('button', { name: look.toUpperCase() })).toHaveAttribute('aria-pressed', 'true');
    await looks.getByRole('button', { name: other.toUpperCase() }).click();
    await expect(board.locator('html')).toHaveAttribute('data-look', other);
    await expect(board.getByRole('region', { name: /^needs you/i })).toBeVisible();
    expect((await eye.api('/api/prefs'))['eye.look']).toBe(other);
    // a new page without ?look= opens in the saved look: the engine's, with this browser's copy gone
    await board.evaluate(() => localStorage.clear());
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
      await board.getByRole('button', { name: /^Accent colour/ }).click();
      await board.getByRole('group', { name: 'Look' }).getByRole('button', { name: name.toUpperCase() }).click();
      await expect(board.locator('html')).toHaveAttribute('data-look', name);
    };
    await board.getByRole('group', { name: 'Layout' }).getByRole('button', { name: 'Grid' }).click();
    await expect(board.locator('.board.gridview')).toBeVisible();
    await pick(other);
    await expect(board.locator('.board.lanes')).toBeVisible();
    // the layout comes back from the engine, not only from this browser
    await board.evaluate(() => localStorage.clear());
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
});

