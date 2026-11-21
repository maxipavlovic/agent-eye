import { spawn } from 'node:child_process';
import { card, done, expect, lane, problems, sid, test } from '../support';

test.describe('board', () => {
  test('loads with every lane and no console problems', async ({ board }) => {
    for (const [name, n] of [['ideas', 3], ['running', 2], ['needs you', 4], ['waiting', 3], ['paused', 2]] as const) {
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
    // a narrow card shows Remote as its icon only; the chip keeps its title
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
    for (const t of ['LEFT BEHIND', 'REMINDER']) await expect(board.getByRole('heading', { name: new RegExp(t) })).toBeVisible();
    for (const b of ['EXPORT']) await expect(board.getByRole('link', { name: b })).toBeVisible();
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
    await card(board, /Fix stale tokens/).click({ button: 'right' });
    const menu = board.getByRole('menu', { name: /Actions for Fix stale tokens/ });
    await menu.getByRole('menuitem', { name: 'CONTINUE TONIGHT…' }).click();
    const form = board.getByRole('form', { name: 'Continue tonight' });
    await expect(form).toBeVisible();
    await expect(form.getByRole('textbox').first()).toBeFocused();
  });
});

test.describe('reminders', () => {
  test('Remind me shows on the card and in the panel', async ({ board, eye }) => {
    await card(board, /Fix the flaky checkout/).click({ button: 'right' });
    await board.getByRole('menuitem', { name: 'TOMORROW 9:00' }).click();
    await expect.poll(async () => (await eye.api(`/api/cards/${sid(8)}`)).remind_at).toBe(Date.UTC(2026, 10, 21, 9));
    await expect(card(board, /Fix the flaky checkout/).getByRole('img', { name: /remind/i })).toBeVisible();
    await card(board, /Fix the flaky checkout/).click();
    const panel = board.getByRole('heading', { name: 'REMINDER' }).locator('..');
    await expect(panel).toContainText(/Sat.*9:00|9:00/);
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

test.describe('left behind', () => {
  test('Ignore asks first, and a yes removes the item', async ({ board, eye }) => {
    await card(board, /Fix stale tokens/).click();
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
    const [download] = await Promise.all([board.waitForEvent('download'), board.getByRole('link', { name: 'EXPORT' }).click()]);
    expect(download.suggestedFilename()).toMatch(/^eye-e2e00000.*\.html$/);
    const path = await download.path();
    const html = (await import('node:fs')).readFileSync(path, 'utf8');
    expect(html).toContain('Fix stale tokens in the account store');
    expect(html).toContain('expire them softly');
    expect(html).not.toContain('ghp_AAAAAAAA');
  });
});
