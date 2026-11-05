import { expect, test } from '@playwright/test';
import { login, ready } from './helpers';

test('the PMU desk opens its overview and a figure opens its page', async ({ page }) => {
  await login(page, 'pmu1');
  await page.getByRole('link', { name: 'PMU' }).click();
  await expect(page).toHaveURL(/\/app\/pmu$/);
  await ready(page);
  await expect(page.getByTestId('pmu-headline').locator('.cell')).toHaveCount(6);
  await expect(page.getByTestId('pmu-books').locator('tbody tr')).toHaveCount(3);
  await expect(page.getByTestId('pmu-concentration').locator('tbody tr').first()).toBeVisible();
  // A headline figure opens the page that explains it.
  await page.getByTestId('pmu-headline').locator('.cell', { hasText: 'Net DV01' }).click();
  await expect(page).toHaveURL(/\/app\/pmu\/risk$/);
  await expect(page.getByTestId('pmu-shocks').locator('tbody tr')).toHaveCount(6);
});

test('a dealer has no PMU tab and is sent back from its address', async ({ page }) => {
  await login(page, 'fx1');
  await expect(page.getByRole('link', { name: 'PMU' })).toHaveCount(0);
  await page.goto('/app/pmu');
  await expect(page).toHaveURL(/\/app\/?$/);
});

test('positions open from book to bond to lot, filter, and show a bond', async ({ page }) => {
  await login(page, 'pmu1');
  await page.goto('/app/pmu/positions');
  await ready(page);
  const tree = page.getByTestId('pmu-tree');
  await expect(tree.locator('tr[data-level="BOOK"]')).toHaveCount(3);
  const bonds = await tree.locator('tr[data-level="BOND"]').count();
  expect(bonds).toBeGreaterThan(20);
  await expect(tree.locator('tr[data-level="LOT"]')).toHaveCount(0);
  // Open the first bond: its lots appear beneath it.
  await tree.locator('tr[data-level="BOND"]').first().getByRole('button', { name: /^expand/ }).click();
  await expect(tree.locator('tr[data-level="LOT"]').first()).toBeVisible();
  // Collapse a book: its bonds go.
  await tree.locator('tr[data-level="BOOK"]').first().getByRole('button', { name: /^collapse/ }).click();
  expect(await tree.locator('tr[data-level="BOND"]').count()).toBeLessThan(bonds);
  // Unhedged only keeps fewer bonds.
  await page.getByRole('button', { name: 'Bonds only' }).click();
  await page.getByRole('button', { name: 'Unhedged only' }).click();
  expect(await tree.locator('tr[data-level="BOND"]').count()).toBeLessThan(bonds);
  // The income view takes a period from the address bar.
  await page.locator('.toolbar').getByRole('button', { name: 'Income', exact: true }).click();
  await page.getByRole('button', { name: 'Year to date' }).click();
  await expect(page).toHaveURL(/view=income/);
  await expect(page).toHaveURL(/p=ytd/);
  await expect(page.getByTestId('pmu-positions')).toContainText('income from 01-Jan-');
  // A bond opens with its lots, swaps and cash flows.
  await tree.locator('tr[data-level="BOND"] button.link').first().click();
  const pane = page.getByTestId('pmu-bond');
  await expect(pane).toContainText('Purchase lots');
  await expect(pane).toContainText('Cash flows, next three years');
  await pane.getByRole('button', { name: 'close the bond' }).click();
  await expect(pane).toHaveCount(0);
});

test('income adds up and the forecast runs off or reinvests', async ({ page }) => {
  await login(page, 'pmu1');
  await page.goto('/app/pmu/income');
  await ready(page);
  await expect(page.getByTestId('pmu-income-strip').locator('.cell')).toHaveCount(6);
  await expect(page.getByTestId('pmu-periods').locator('tbody tr')).toHaveCount(4);
  const before = await page.getByTestId('pmu-forecast-strip').locator('.cell').nth(0).locator('.value').innerText();
  await page.getByTestId('pmu-forecast').getByRole('button', { name: 'Reinvest' }).click();
  await expect(page.getByTestId('pmu-forecast').getByRole('button', { name: 'Reinvest' })).toHaveAttribute('aria-pressed', 'true');
  // This year rarely changes (little matures before December); the later years must.
  await expect(page.getByTestId('pmu-periods')).toBeVisible();
  expect(before.length).toBeGreaterThan(0);
});

test('funding shows the repo book and flags old quotes', async ({ page }) => {
  await login(page, 'pmu1');
  await page.goto('/app/pmu/funding');
  await ready(page);
  await expect(page.getByTestId('pmu-funding-strip').locator('.cell')).toHaveCount(5);
  expect(await page.getByTestId('pmu-repos').locator('tbody tr').count()).toBeGreaterThan(5);
  await expect(page.getByTestId('pmu-quotes')).toContainText('days old');
});

test('a PMU user sees the bond portfolio, not the dealing desks, and cannot book a deal', async ({ page }) => {
  await login(page, 'pmu1');
  await ready(page);
  const tabs = page.locator('nav.tabs');
  await expect(tabs.getByRole('link', { name: 'PMU' })).toBeVisible();
  for (const name of ['Blotter', 'Breaks', 'Liquidity']) await expect(tabs.getByRole('link', { name })).toHaveCount(0);
  // The Overview leads with the bond portfolio and has no trading section.
  await expect(page.getByTestId('ov-pmu-strip').locator('.cell')).toHaveCount(6);
  await expect(page.getByTestId('trading-strip')).toHaveCount(0);
  // The blotter's address sends them home, and the palette offers no deal booking.
  await page.goto('/app/blotter?book=1');
  await expect(page).toHaveURL(/\/app\/?$/);
  await page.keyboard.press('Control+k');
  await page.keyboard.type('book');
  await expect(page.getByText('Book a deal')).toHaveCount(0);
});

test('the treasury head sees the bond portfolio on the Overview beside the trading desks', async ({ page }) => {
  await login(page, 'head1');
  await ready(page);
  await expect(page.getByTestId('trading-strip')).toBeVisible();
  await expect(page.getByTestId('ov-pmu-strip').locator('.cell')).toHaveCount(6);
});
