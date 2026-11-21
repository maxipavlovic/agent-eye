// Screenshots of the real page: the board in both layouts, the panel, the menu, the idea form and
// the usage panel, light and dark, at two widths. Baselines are made by `scripts/e2e.sh --update`
// in the pinned Playwright container (see docs/testing.md). The server clock, the browser clock
// and the time zone are fixed, so what the page says about time is the same on every run.
import type { Page } from '@playwright/test';
import { card, expect, test } from '../support';

const SCHEMES = ['light', 'dark'] as const;
const WIDTHS = [1440, 1000] as const;

const layout = (p: Page, name: 'Lanes' | 'Grid') => p.getByRole('group', { name: 'Layout' }).getByRole('button', { name }).click();

const VIEWS: Record<string, (p: Page) => Promise<void>> = {
  lanes: async (p) => { await layout(p, 'Lanes'); },
  grid: async (p) => { await layout(p, 'Grid'); },
  panel: async (p) => {
    await card(p, /Fix stale tokens/).click();
    await expect(p.getByRole('heading', { level: 2, name: /Fix stale tokens/ })).toBeVisible();
    await expect(p.getByRole('heading', { name: /LEFT BEHIND/ })).toBeVisible();
  },
  menu: async (p) => {
    // a card that is fully in view at 1000 px, so the click does not scroll the lanes
    await card(p, /Fix stale tokens/).click({ button: 'right' });
    await expect(p.getByRole('menu')).toBeVisible();
  },
  'idea-form': async (p) => {
    await p.getByRole('button', { name: '+ IDEA' }).click();
    await expect(p.getByRole('form', { name: 'New idea' })).toBeVisible();
  },
  usage: async (p) => {
    await p.getByRole('button', { name: 'USAGE' }).click();
    await expect(p.getByRole('heading', { name: /LAST 4 WEEKS/ })).toBeVisible();
  },
};

for (const scheme of SCHEMES) {
  for (const width of WIDTHS) {
    for (const [name, open] of Object.entries(VIEWS)) {
      test(`${name} ${scheme} ${width}`, async ({ board }) => {
        await board.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
        await board.setViewportSize({ width, height: 900 });
        await open(board);
        await board.evaluate(() => document.fonts.ready);
        // the connection chip and the header eye follow the live connection and the pointer
        await board.mouse.move(width - 5, 895);
        await expect(board).toHaveScreenshot(`${name}-${scheme}-${width}.png`, {
          mask: [board.locator('.brand'), board.locator('.conn')],
        });
      });
    }
  }
}
