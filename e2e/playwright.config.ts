import { defineConfig, devices } from '@playwright/test';

// The board's looks; every flow and every screenshot runs in each of them. Iris draws its cards in as the board
// arrives, so its pages ask for reduced motion and a click never lands on a card that is still moving (ink runs
// with motion: its screenshots freeze the pulse at its first frame). Reduced motion is a context option; as a
// plain `use` option Playwright ignores it.
const IRIS = { look: 'iris', contextOptions: { reducedMotion: 'reduce' } } as const;

// Two engines times two looks (ink, iris), one viewport; the visual tests set their own widths. Snapshots live in
// __screenshots__/<project>/ and are made only in the pinned Playwright container
// (scripts/e2e.sh), so fonts and rendering are the same on every machine.
export default defineConfig<{ look: string }>({
  testDir: './tests',
  snapshotPathTemplate: '{testDir}/../__screenshots__/{projectName}/{arg}{ext}',
  fullyParallel: true,
  workers: process.env.CI ? 2 : 4,
  retries: 0,
  timeout: 30_000,
  expect: {
    timeout: 8_000,
    toHaveScreenshot: { animations: 'disabled', caret: 'hide', maxDiffPixelRatio: 0.002 },
  },
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
  outputDir: 'test-results',
  use: {
    viewport: { width: 1440, height: 900 },
    timezoneId: 'UTC',
    locale: 'en-US',
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } } },
    { name: 'webkit', use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 900 } } },
    { name: 'chromium-iris', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 }, ...IRIS } },
    { name: 'webkit-iris', use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 900 }, ...IRIS } },
  ],
});
