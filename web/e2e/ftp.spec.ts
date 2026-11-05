import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { login, ready } from './helpers';

test('every desk sees the FTP curves in force from the status bar', async ({ page }) => {
  await login(page, 'fx1');
  await ready(page);
  const chip = page.getByTestId('ftp-chip');
  await expect(chip).toContainText(/FTP .* 4 curves/);
  await chip.click();
  await expect(page).toHaveURL(/\/app\/ftp$/);
  await ready(page);
  const table = page.getByTestId('ftp-table');
  for (const ccy of ['BHD', 'USD', 'AED', 'SAR']) await expect(table.locator('th', { hasText: ccy })).toBeVisible();
  await expect(table.locator('tbody tr')).toHaveCount(11);
  // A dealer reads the curves and is not offered publishing.
  await expect(page.getByRole('link', { name: 'Publish new curves' })).toHaveCount(0);
});

test('ALM drops the workbook, sees the preview and publishes it to every desk', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/ftp');
  await ready(page);
  const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Download the template' }).click()]);
  expect(dl.suggestedFilename()).toMatch(/^ftp-curves-\d{4}-\d{2}-\d{2}\.xlsx$/);
  const file = await dl.path();
  await page.getByLabel('FTP workbook').setInputFiles({ name: 'FTP curves.xlsx', mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', buffer: readFileSync(file) });
  const preview = page.getByTestId('ftp-preview');
  await expect(preview).toContainText('ready to publish');
  await expect(preview).toContainText('4 curves, read as one sheet, a column per currency');
  const publish = page.getByRole('button', { name: 'Publish to every desk' });
  await expect(publish).toBeDisabled();
  await page.getByLabel('Publication note').fill('End-to-end check: the curves in force, republished');
  await publish.click();
  await expect(page.getByText(/FTP curves published for/)).toBeVisible();
  await page.goto('/app/ftp');
  await ready(page);
  await expect(page.getByTestId('ftp-recent').locator('tbody tr').first()).toContainText('ALM Analyst');
});

test('a file that is not a workbook is refused with the reason', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/ftp');
  await ready(page);
  await page.getByLabel('FTP workbook').setInputFiles({ name: 'notes.xlsx', mimeType: 'application/octet-stream', buffer: Buffer.from('not a workbook') });
  await expect(page.getByTestId('ftp-errors')).toContainText('not an Excel workbook');
  await expect(page.getByRole('button', { name: 'Publish to every desk' })).toBeDisabled();
});
