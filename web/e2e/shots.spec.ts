// Screenshots of every page in both themes, for the layout review. Writes e2e/shots/<name>-<theme>.png
// (or SHOTS_DIR). Tagged @shots so the ordinary e2e run skips them.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { test } from '@playwright/test';
import { login, ready, setTheme } from './helpers';

const dir = process.env.SHOTS_DIR ?? join('e2e', 'shots');
const routes: [string, string][] = [
  ['overview', '/app/'],
  ['blotter', '/app/blotter'],
  ['breaks', '/app/breaks'],
  ['liquidity', '/app/liquidity'],
  ['alm', '/app/alm'],
];

for (const theme of ['light', 'dark'] as const) {
  test(`@shots pages in ${theme}`, async ({ page }) => {
    mkdirSync(dir, { recursive: true });
    await setTheme(page, theme);
    await page.goto('/app/');
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(dir, `login-${theme}.png`) });
    await login(page, process.env.SHOTS_USER ?? 'alm1');
    for (const [name, path] of routes) {
      await page.goto(path);
      await ready(page);
      await page.screenshot({ path: join(dir, `${name}-${theme}.png`), fullPage: true });
    }
    await page.goto('/app/');
    await ready(page);
    await page.locator('.bell').click();
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(dir, `bell-${theme}.png`) });
  });
}
