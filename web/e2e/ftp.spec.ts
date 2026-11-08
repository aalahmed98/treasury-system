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
  await expect(page.getByTestId('ftp-publish-log').locator('tbody tr').first()).toContainText('ALM Analyst');
  await expect(page.getByTestId('ftp-publish-log').locator('tbody tr').first()).toContainText('in force');
});

test('a second workbook for the day replaces the first, and the log says who replaced whom', async ({ browser }) => {
  // ALM publishes the template; the head drops a copy and is told whose curves it replaces.
  const alm = await browser.newPage();
  await login(alm, 'alm1');
  await alm.goto('/app/alm/ftp');
  await ready(alm);
  const [dl] = await Promise.all([alm.waitForEvent('download'), alm.getByRole('button', { name: 'Download the template' }).click()]);
  const buffer = readFileSync(await dl.path());
  const xlsx = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
  await alm.getByLabel('FTP workbook').setInputFiles({ name: 'FTP morning.xlsx', mimeType: xlsx, buffer });
  await expect(alm.getByTestId('ftp-preview')).toContainText('ready to publish');
  await alm.getByLabel('Publication note').fill('End-to-end: the morning curves');
  await alm.getByRole('button', { name: 'Publish to every desk' }).click();
  await expect(alm.getByText(/FTP curves published for/)).toBeVisible();

  const head = await browser.newPage();
  await login(head, 'head1');
  await head.goto('/app/alm/ftp');
  await ready(head);
  await head.getByLabel('FTP workbook').setInputFiles({ name: 'FTP corrected.xlsx', mimeType: xlsx, buffer });
  await expect(head.getByTestId('ftp-replaces')).toContainText('ALM Analyst');
  await expect(head.getByTestId('ftp-replaces')).toContainText('FTP morning.xlsx');
  await head.getByLabel('Publication note').fill('End-to-end: corrected by the head');
  await head.getByRole('button', { name: 'Publish to every desk' }).click();
  await expect(head.getByText(/FTP curves published for/)).toBeVisible();

  // A dealer sees both in the log, newest first, and downloads the workbook in force.
  const fx = await browser.newPage();
  await login(fx, 'fx1');
  await fx.goto('/app/ftp');
  await ready(fx);
  const rows = fx.getByTestId('ftp-log').locator('tbody tr');
  await expect(rows.nth(0)).toContainText('Head of Treasury');
  await expect(rows.nth(0)).toContainText('in force');
  await expect(rows.nth(0)).toContainText('replacing ALM Analyst');
  await expect(rows.nth(1)).toContainText('FTP morning.xlsx');
  await expect(rows.nth(1)).toContainText('replaced');
  await expect(rows.nth(1)).toContainText('by Head of Treasury');
  const [file] = await Promise.all([fx.waitForEvent('download'), rows.nth(0).getByRole('button', { name: 'FTP corrected.xlsx' }).click()]);
  expect(file.suggestedFilename()).toBe('FTP corrected.xlsx');
  expect(readFileSync(await file.path()).equals(buffer)).toBe(true);
  await Promise.all([alm.close(), head.close(), fx.close()]);
});

test('a file that is not a workbook is refused with the reason', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/ftp');
  await ready(page);
  await page.getByLabel('FTP workbook').setInputFiles({ name: 'notes.xlsx', mimeType: 'application/octet-stream', buffer: Buffer.from('not a workbook') });
  await expect(page.getByTestId('ftp-errors')).toContainText('not an Excel workbook');
  await expect(page.getByRole('button', { name: 'Publish to every desk' })).toBeDisabled();
});
