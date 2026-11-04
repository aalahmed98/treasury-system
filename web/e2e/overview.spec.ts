import { expect, test } from '@playwright/test';
import { login, ready } from './helpers';

test('an ALM user sees the balance-sheet strip with limits and charts', async ({ page }) => {
  await login(page, 'alm1');
  await ready(page);
  await expect(page.getByTestId('alm-strip')).toBeVisible();
  await expect(page.getByTestId('alm-strip').locator('.cell')).toHaveCount(4);
  await expect(page.getByTestId('alm-strip-2').locator('.cell')).toHaveCount(4);
  await expect(page.getByText('Limits', { exact: true })).toBeVisible();
  await expect(page.locator('svg.chart').first()).toBeVisible();
});

test('a trader gets the trading strip but no balance sheet', async ({ page }) => {
  await login(page, 'fx1');
  await ready(page);
  await expect(page.getByTestId('trading-strip').locator('.cell')).toHaveCount(6);
  await expect(page.getByTestId('alm-strip')).toHaveCount(0);
});

test('a balance-sheet cell opens its ALM tool', async ({ page }) => {
  await login(page, 'alm1');
  await ready(page);
  await page.getByTestId('alm-strip').locator('.cell').first().click();
  await expect(page).toHaveURL(/\/app\/alm\/lcr$/);
});
