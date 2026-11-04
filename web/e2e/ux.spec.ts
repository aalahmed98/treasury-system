import { expect, test } from '@playwright/test';
import { login, ready } from './helpers';

test('the command palette jumps to a deal by reference', async ({ page }) => {
  await login(page, 'fx1');
  await ready(page);
  await page.keyboard.press('Control+k');
  const box = page.getByRole('combobox', { name: 'Command' });
  await expect(box).toBeVisible();
  await box.fill('FX-0002');
  const first = page.getByRole('option').first();
  await expect(first).toContainText('FX-0002');
  await first.click();
  await expect(page).toHaveURL(/\/app\/blotter\?deal=/);
  await expect(page.getByTestId('deal-detail')).toBeVisible();
});

test('g then k goes to the breaks page; ? lists the shortcuts', async ({ page }) => {
  await login(page, 'fx1');
  await ready(page);
  await page.keyboard.press('g');
  await page.keyboard.press('k');
  await expect(page).toHaveURL(/\/app\/breaks$/);
  await page.keyboard.press('?');
  await expect(page.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Keyboard shortcuts' })).toHaveCount(0);
});

test('arrow keys move the blotter selection and the address follows', async ({ page }) => {
  await login(page, 'fx1');
  await page.goto('/app/blotter');
  const table = page.getByTestId('blotter');
  await expect(table.locator('tbody tr[data-key]').first()).toBeVisible();
  await table.focus();
  await page.keyboard.press('ArrowDown');
  await expect(page).toHaveURL(/deal=/);
  const firstKey = await table.locator('tbody tr[data-key]').first().getAttribute('data-key');
  expect(page.url()).toContain(`deal=${firstKey}`);
  await page.keyboard.press('ArrowDown');
  const secondKey = await table.locator('tbody tr[data-key]').nth(1).getAttribute('data-key');
  expect(page.url()).toContain(`deal=${secondKey}`);
  await expect(table.locator('tr.selected')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(page).not.toHaveURL(/deal=/);
});

test('booking from the side sheet shows a toast and selects the new deal', async ({ page }) => {
  await login(page, 'fx1');
  await page.goto('/app/blotter');
  await expect(page.getByTestId('blotter').locator('tbody tr[data-key]').first()).toBeVisible();
  await page.keyboard.press('n');
  const sheet = page.getByRole('dialog', { name: 'Book a deal' });
  await expect(sheet).toBeVisible();
  await expect(page.getByTestId('blotter')).toBeVisible();
  await sheet.getByLabel('Counterparty').fill('Palette Bank');
  await sheet.getByRole('button', { name: 'Book', exact: true }).click();
  await expect(page.getByTestId('toast')).toContainText('Booked FX-');
  await expect(page.getByTestId('deal-detail')).toContainText('Palette Bank');
});

test('the positions panel closes, stays closed on reload and comes back from the toolbar', async ({ page }) => {
  await login(page, 'fx1');
  await page.goto('/app/blotter');
  await expect(page.getByTestId('blotter')).toBeVisible();
  await page.getByRole('button', { name: 'hide positions' }).click();
  await expect(page.getByRole('region', { name: 'Positions' })).toHaveCount(0);
  await page.reload();
  await expect(page.getByTestId('blotter')).toBeVisible();
  await expect(page.getByRole('region', { name: 'Positions' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Positions' }).click();
  await expect(page.getByRole('region', { name: 'Positions' })).toBeVisible();
});

test('the overview leads with the attention list and its rows navigate', async ({ page }) => {
  await login(page, 'alm1');
  await ready(page);
  const attention = page.getByTestId('attention');
  await expect(attention).toBeVisible();
  const row = attention.locator('.attention-row', { hasText: /mismatch|ambiguous|orphan|provisional/ });
  if ((await row.count()) > 0) {
    await row.first().click();
    await expect(page).toHaveURL(/\/app\/(blotter|breaks)/);
  }
});

test('the ALM assumptions rail closes and the what-if lists its changes', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/lcr');
  await ready(page);
  const input = page.locator('.assumptions input[type=number]').nth(1);
  await input.fill('115');
  await page.getByRole('button', { name: /what-if · 1 change/ }).click();
  await expect(page.getByRole('dialog', { name: 'What changed' })).toContainText('115');
  await page.getByRole('button', { name: 'revert' }).click();
  await expect(page.getByText(/what-if · 0 changes/)).toBeVisible();
  await page.getByRole('button', { name: 'Reset' }).click();
  await page.getByRole('button', { name: 'hide assumptions' }).click();
  await expect(page.locator('.assumptions')).toHaveCount(0);
  await page.getByRole('button', { name: 'Assumptions' }).click();
  await expect(page.locator('.assumptions')).toBeVisible();
});

test('several breaks can be picked with Ctrl-click and acknowledged at once', async ({ page }) => {
  await login(page, 'fx1');
  await page.goto('/app/breaks');
  const table = page.getByTestId('breaks');
  const rows = table.locator('tbody tr[data-key]');
  await expect(rows.first()).toBeVisible();
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ['Control'] });
  await rows.nth(2).click({ modifiers: ['Control'] });
  const bulk = page.getByTestId('bulk-actions');
  await expect(bulk).toContainText('3 breaks selected');
  await expect(table.locator('tr.checked')).toHaveCount(3);
  await page.keyboard.press('Escape');
  await expect(bulk).toHaveCount(0);
});
