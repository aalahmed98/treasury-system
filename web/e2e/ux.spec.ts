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
  await expect(page).toHaveURL(new RegExp(`deal=${firstKey}`));
  const secondKey = await table.locator('tbody tr[data-key]').nth(1).getAttribute('data-key');
  await page.keyboard.press('ArrowDown');
  await expect(page).toHaveURL(new RegExp(`deal=${secondKey}`));
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

test('the FX risk page shows positions, P&L, limits and VaR for a dealer', async ({ page }) => {
  await login(page, 'fx1');
  await page.getByRole('link', { name: /^Risk/ }).click();
  await expect(page.getByTestId('risk-page')).toBeVisible();
  await expect(page.getByTestId('risk-hero').locator('.cell')).toHaveCount(4);
  await expect(page.getByTestId('positions').locator('tbody tr').first()).toBeVisible();
  await expect(page.getByTestId('pnl-dealer')).toBeVisible();
  await expect(page.getByTestId('counterparties')).toBeVisible();
  await expect(page.locator('.section', { hasText: 'Value at risk' })).toBeVisible();
  // A dealer reads the limits but cannot change them.
  await page.getByRole('link', { name: 'Limits' }).click();
  await expect(page.getByTestId('limits-page')).toContainText('Read only');
  await expect(page.getByRole('button', { name: /Start from version/ })).toHaveCount(0);
});

test('the money-market desk has no FX risk tab or page', async ({ page }) => {
  await login(page, 'mm1');
  await expect(page.getByRole('link', { name: /^Risk/ })).toHaveCount(0);
  await page.goto('/app/risk/limits');
  await expect(page).not.toHaveURL(/risk/);
});

test('the treasury head drafts limits and the save box lists the change', async ({ page }) => {
  await login(page, 'head1');
  await page.goto('/app/risk/limits');
  await page.getByRole('button', { name: /Start from version/ }).click();
  const box = page.getByTestId('limits-save-box');
  await expect(box).toContainText('Nothing differs');
  await page.locator('label.arow', { hasText: 'Stop-loss, one day' }).locator('input').fill('175000');
  await expect(box).toContainText('stop loss daily');
  await expect(box.getByRole('button', { name: /Save as version/ })).toBeDisabled();
  await box.getByRole('button', { name: 'Discard draft' }).click();
});

test('the ticket shows the market mid and asks before booking off-market', async ({ page }) => {
  await login(page, 'fx1');
  await page.goto('/app/blotter');
  await expect(page.getByTestId('blotter').locator('tbody tr[data-key]').first()).toBeVisible();
  await page.keyboard.press('n');
  const sheet = page.getByRole('dialog', { name: 'Book a deal' });
  await expect(sheet.getByTestId('market-check')).toContainText(/market 0\.376\d\d for the value date/);
  await sheet.getByLabel('Rate').fill('0.39');
  await expect(sheet.getByTestId('market-check')).toContainText('beyond the');
  await sheet.getByLabel('Counterparty').fill('Off Market Bank');
  await expect(sheet.getByRole('button', { name: 'Book', exact: true })).toBeDisabled();
  await sheet.getByText(/Book at this rate although/).click();
  await expect(sheet.getByRole('button', { name: 'Book', exact: true })).toBeEnabled();
  await sheet.getByRole('button', { name: 'Discard' }).click();
});

test('the ALM workbench has the ladder, early warnings and the ALCO pack', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/ladder');
  await ready(page);
  await expect(page.getByTestId('headline')).toContainText('cumulative gap to one month');
  await expect(page.getByTestId('liquidity-ladder').locator('tbody tr')).toHaveCount(9);
  await page.getByRole('button', { name: 'Contractual' }).click();
  await expect(page.getByTestId('headline')).toContainText('contractual');
  await page.goto('/app/alm/ewi');
  await ready(page);
  await expect(page.getByTestId('headline')).toContainText('contingency plan stage');
  await expect(page.locator('.ewi-row')).toHaveCount(8);
  await page.goto('/app/alm/stress');
  await ready(page);
  await expect(page.getByTestId('uniform-200')).toContainText('All curves +200 bp');
  await page.goto('/app/alm/pack');
  await ready(page);
  await expect(page.getByTestId('alco-pack')).toContainText('ALCO pack');
  await expect(page.getByTestId('pack-signoff')).toContainText(/Awaiting the treasury head|Signed off/);
  const note = page.getByLabel('Commentary on headline');
  if (await note.count()) {
    await note.fill(`Playwright note ${Date.now()}`);
    await page.getByLabel('Commentary on limits').click();
    await expect(page.locator('.pack-note').first()).toContainText('last edited by');
  }
});
