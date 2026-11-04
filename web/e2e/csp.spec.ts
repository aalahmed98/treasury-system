import { expect, test } from '@playwright/test';
import { login, ready } from './helpers';

test('the pages run clean under the content security policy', async ({ page }) => {
  const violations: string[] = [];
  page.on('console', (m) => {
    if (/Content Security Policy|Refused to/i.test(m.text())) violations.push(m.text());
  });
  await login(page, 'alm1');
  for (const path of ['/app/', '/app/blotter', '/app/breaks', '/app/liquidity', '/app/alm/lcr', '/app/alm/stress']) {
    await page.goto(path);
    await ready(page);
  }
  expect(violations).toEqual([]);
  // The remembered theme still applies before the first paint, now from a file.
  await page.evaluate(() => localStorage.setItem('nbb-theme', 'dark'));
  await page.goto('/app/');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});
