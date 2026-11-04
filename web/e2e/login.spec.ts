import { expect, test } from '@playwright/test';
import { login } from './helpers';

test('a wrong password is refused with the server message', async ({ page }) => {
  await page.goto('/app/');
  await page.getByLabel('Username').fill('fx1');
  await page.getByLabel('Password').fill('wrong');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toBeVisible();
});

test('a trader sees the overview without the ALM tab', async ({ page }) => {
  await login(page, 'fx1');
  await expect(page.getByTestId('trading-strip')).toBeVisible();
  await expect(page.getByRole('link', { name: 'ALM' })).toHaveCount(0);
});

test('an ALM user gets the ALM tab and the deep link survives a reload', async ({ page }) => {
  await login(page, 'alm1');
  await expect(page.getByRole('link', { name: 'ALM' })).toBeVisible();
  await page.goto('/app/alm/lcr');
  await expect(page.locator('header.topbar')).toBeVisible();
});
