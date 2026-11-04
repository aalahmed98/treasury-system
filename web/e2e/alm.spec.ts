import { expect, test } from '@playwright/test';
import { login, ready } from './helpers';

const TOOLS = ['lcr', 'nsfr', 'gap', 'nii', 'stress', 'attribution', 'ird', 'funding'];

test('every ALM tool renders its headline for an ALM user', async ({ page }) => {
  await login(page, 'alm1');
  for (const t of TOOLS) {
    await page.goto(`/app/alm/${t}`);
    await ready(page);
    await expect(page.getByTestId('headline')).toBeVisible();
  }
});

test('an edited assumption becomes a what-if and reset restores the bank set', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/lcr');
  await ready(page);
  await expect(page.getByText(/bank v\d+/)).toBeVisible();
  const input = page.locator('.assumptions input[type=number]').nth(1);
  await input.fill('115');
  await expect(page.getByText(/what-if · 1 change/)).toBeVisible();
  await page.getByRole('button', { name: 'Reset' }).click();
  await expect(page.getByText(/bank v\d+/)).toBeVisible();
});

test('setup lists the versions and opens a draft', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/setup');
  await ready(page);
  await expect(page.getByText('Versions', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: /Start from version \d+/ }).click();
  await expect(page.getByTestId('draft-editor')).toBeVisible();
  await expect(page.getByRole('button', { name: /Save as version \d+/ })).toBeDisabled();
});

test('a trader is sent back to the overview from the ALM address', async ({ page }) => {
  await login(page, 'fx1');
  await page.goto('/app/alm/lcr');
  await expect(page).toHaveURL(/\/app\/?$/);
});
