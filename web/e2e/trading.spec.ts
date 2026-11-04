import { expect, test } from '@playwright/test';
import { login } from './helpers';

test('the blotter lists deals, filters and opens a detail with provenance', async ({ page }) => {
  await login(page, 'fx1');
  await page.getByRole('link', { name: 'Blotter' }).click();
  const table = page.getByTestId('blotter');
  await expect(table.locator('tbody tr[data-key]').first()).toBeVisible();
  await page.getByLabel('Search deals').fill('BREAK');
  await expect(table.locator('tbody tr[data-key]').first()).toContainText('BREAK');
  await table.locator('tbody tr[data-key]').first().click();
  await expect(page.getByTestId('deal-detail')).toBeVisible();
  await page.getByRole('button', { name: 'Sources' }).click();
  await expect(page.getByText('Counterparty (as sent → canonical)')).toBeVisible();
});

test('a trader books a deal and sees it in the blotter', async ({ page }) => {
  await login(page, 'fx1');
  await page.getByRole('link', { name: 'Blotter' }).click();
  await page.getByRole('button', { name: 'Book deal' }).click();
  const dialog = page.getByRole('dialog', { name: 'Book a deal' });
  await dialog.getByLabel('Counterparty').fill('Playwright Bank');
  await dialog.getByRole('button', { name: 'Book', exact: true }).click();
  await expect(dialog).toBeHidden();
  await page.getByLabel('Search deals').fill('playwright bank');
  await expect(page.getByTestId('blotter').locator('tbody tr[data-key]').first()).toContainText('Playwright Bank');
});

test('breaks list with actions for the desk', async ({ page }) => {
  await login(page, 'fx1');
  await page.getByRole('link', { name: 'Breaks' }).click();
  const table = page.getByTestId('breaks');
  await expect(table.locator('tbody tr[data-key]').first()).toBeVisible();
  await table.locator('tbody tr[data-key]').first().click();
  await expect(page.getByTestId('break-actions')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Dismiss' })).toBeVisible();
});

test('the ladder shows the currency chips and a date drill-down', async ({ page }) => {
  await login(page, 'head1');
  await page.getByRole('link', { name: 'Liquidity' }).click();
  await expect(page.getByTestId('ladder').locator('tbody tr').first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'USD', exact: true })).toBeVisible();
  await page.getByTestId('ladder').locator('tbody tr').nth(2).click();
  await expect(page.getByText('Expected flows', { exact: true })).toBeVisible();
});
