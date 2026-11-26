// Screenshots of the real page: the board in both layouts, the panel, an idea's panel, the menu, the idea form and
// the usage and settings panels, light and dark, at two widths. Baselines are made by `scripts/e2e.sh --update`
// in the pinned Playwright container (see docs/testing.md). The server clock, the browser clock
// and the time zone are fixed, so what the page says about time is the same on every run.
import type { Page } from '@playwright/test';
import { demoClock } from '../../face/src/data/clockSample';
import { card, expect, test } from '../support';

const SCHEMES = ['light', 'dark'] as const;
const WIDTHS = [1440, 1000] as const;

// the layout lives in the settings panel: open it, press, and close it again
const layout = async (p: Page, name: 'Lanes' | 'Grid') => {
  await p.getByRole('button', { name: 'Settings', exact: true }).click();
  await p.getByRole('group', { name: 'Layout' }).getByRole('button', { name }).click();
  await p.keyboard.press('Escape');
  await expect(p.getByRole('complementary', { name: 'Settings' })).toHaveCount(0);
};

const VIEWS: Record<string, (p: Page) => Promise<void>> = {
  lanes: async (p) => { await layout(p, 'Lanes'); },
  grid: async (p) => { await layout(p, 'Grid'); },
  panel: async (p) => {
    await card(p, /Fix stale tokens/).click();
    await expect(p.getByRole('heading', { level: 2, name: /Fix stale tokens/ })).toBeVisible();
    await expect(p.getByRole('heading', { name: /LEFTOVERS/ })).toBeVisible();
  },
  menu: async (p) => {
    // a card that is fully in view at 1000 px, so the click does not scroll the lanes
    await card(p, /Fix stale tokens/).click({ button: 'right' });
    await expect(p.getByRole('menu')).toBeVisible();
  },
  'idea-panel': async (p) => {
    await card(p, /Write a changelog generator/).click();
    await expect(p.getByRole('heading', { level: 2, name: /Write a changelog generator/ })).toBeVisible();
  },
  'idea-form': async (p) => {
    await p.getByRole('button', { name: '+ IDEA' }).click();
    await expect(p.getByRole('form', { name: 'New idea' })).toBeVisible();
  },
  settings: async (p) => {
    await p.getByRole('button', { name: 'Settings', exact: true }).click();
    await expect(p.getByRole('heading', { name: 'AGENTS' })).toBeVisible();
  },
  hours: async (p) => {
    await p.route('**/api/clock*', (r) => r.fulfill({ contentType: 'application/json', body: JSON.stringify(demoClock()) }));
    await p.getByRole('button', { name: 'HOURS', exact: true }).click();
    await expect(p.locator('.hours .cdiagram')).toBeVisible();
  },
  usage: async (p) => {
    await p.getByRole('button', { name: 'USAGE' }).click();
    await expect(p.getByRole('heading', { name: /LAST 4 WEEKS/ })).toBeVisible();
  },
};

// The page is made at its width and scheme (not resized afterwards), so nothing re-lays out under
// the screenshot.
for (const scheme of SCHEMES) {
  for (const width of WIDTHS) {
    test.describe(`${scheme} ${width}`, () => {
      test.use({ colorScheme: scheme, viewport: { width, height: 900 } });
      for (const [name, open] of Object.entries(VIEWS)) {
        test(name, async ({ board }) => {
          await open(board);
          await board.waitForLoadState('networkidle');
          await board.evaluate(() => document.fonts.ready);
          await board.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
          // the pointer rests in a corner, away from any hover state
          await board.mouse.move(width - 5, 895);
          // the connection chip and the header eye follow the live connection and the pointer
          await expect(board).toHaveScreenshot(`${name}-${scheme}-${width}.png`, {
            mask: [board.locator('.brand'), board.locator('.conn')],
          });
        });
      }
    });
  }
}

// The card's frame holds whatever a card carries: the worst-case card (Needs you, a reminder, a night run,
// leftovers and Remote Control, an app origin, a long project name and a long title) at three widths, light
// and dark. A new state becomes a mark on the top edge or goes to the panel, never a new row or chip; if a
// change breaks that, these screenshots show it.
for (const scheme of SCHEMES) {
  test.describe(`worst-case card ${scheme}`, () => {
    test.use({ colorScheme: scheme, viewport: { width: 1440, height: 900 } });
    for (const w of [260, 310, 360]) {
      test(`${w} px`, async ({ board }) => {
        const c = card(board, /Rework the customer billing reconciliation/);
        await expect(c).toBeVisible();
        await c.evaluate((el, px) => { (el as HTMLElement).style.width = `${px}px`; }, w);
        await board.evaluate(() => document.fonts.ready);
        await board.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
        await board.mouse.move(1435, 895);
        await expect(c).toHaveScreenshot(`worst-card-${scheme}-${w}.png`);
      });
    }
  });
}
