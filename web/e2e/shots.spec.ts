// Screenshots of every page in both themes, for the layout review. Writes e2e/shots/<name>-<theme>.png
// (or SHOTS_DIR). Tagged @shots so the ordinary e2e run skips them.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { test } from '@playwright/test';
import { fullPage, login, ready, setTheme } from './helpers';

const dir = process.env.SHOTS_DIR ?? join('e2e', 'shots');
const routes: [string, string][] = [
  ['overview', '/app/'],
  ['blotter', '/app/blotter'],
  ['breaks', '/app/breaks'],
  ['liquidity', '/app/liquidity'],
  ['alm', '/app/alm'],
  ['alm-lcr', '/app/alm/lcr'],
  ['alm-nsfr', '/app/alm/nsfr'],
  ['alm-gap', '/app/alm/gap'],
  ['alm-nii', '/app/alm/nii'],
  ['alm-eve', '/app/alm/eve'],
  ['alm-stress', '/app/alm/stress'],
  ['alm-attribution', '/app/alm/attribution'],
  ['alm-ird', '/app/alm/ird'],
  ['alm-funding', '/app/alm/funding'],
  ['alm-setup', '/app/alm/setup'],
  ['alm-ladder', '/app/alm/ladder'],
  ['alm-ewi', '/app/alm/ewi'],
  ['alm-pack', '/app/alm/pack'],
  ['ftp', '/app/ftp'],
  ['pmu', '/app/pmu'],
  ['pmu-positions', '/app/pmu/positions'],
  ['pmu-income', '/app/pmu/income'],
  ['pmu-risk', '/app/pmu/risk'],
  ['pmu-funding', '/app/pmu/funding'],
  ['alm-ftp', '/app/alm/ftp'],
];

for (const theme of ['light', 'dark'] as const) {
  test(`@shots pages in ${theme}`, async ({ page }) => {
    mkdirSync(dir, { recursive: true });
    await setTheme(page, theme);
    await page.goto('/app/');
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(dir, `login-${theme}.png`) });
    await login(page, process.env.SHOTS_USER ?? 'head1');
    for (const [name, path] of routes) {
      await page.goto(path);
      await ready(page);
      await fullPage(page, join(dir, `${name}-${theme}.png`));
    }
    for (const [name, path] of [
      ['risk', '/app/risk'],
      ['risk-limits', '/app/risk/limits'],
    ] as const) {
      await page.goto(path);
      await ready(page);
      await page.waitForTimeout(800);
      await fullPage(page, join(dir, `${name}-${theme}.png`));
    }
    await page.goto('/app/');
    await ready(page);
    await page.locator('.bell').click();
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(dir, `bell-${theme}.png`) });
    await page.keyboard.press('Escape');
    await page.keyboard.press('Control+k');
    await page.getByRole('combobox', { name: 'Command' }).fill('qatar');
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(dir, `palette-${theme}.png`) });
    await page.keyboard.press('Escape');
    await page.goto('/app/blotter');
    await ready(page);
    await page.getByTestId('blotter').locator('tbody tr[data-key]').first().click();
    await page.waitForTimeout(400);
    await page.screenshot({ path: join(dir, `blotter-detail-${theme}.png`) });
    await page.goto('/app/breaks');
    await ready(page);
    await page.getByTestId('breaks').locator('tbody tr[data-key]').first().click();
    await page.waitForTimeout(400);
    await page.screenshot({ path: join(dir, `breaks-detail-${theme}.png`) });
  });
}
