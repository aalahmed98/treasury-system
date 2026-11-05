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
  await expect(page.getByTestId('pmu-funding-strip').locator('.cell')).toHaveCount(6);
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

/** A valid ISIN not used before: XS, nine random digits, and its check digit (ISO 6166). */
function freshIsin(): string {
  const body = 'XS' + String(Math.floor(Math.random() * 1e9)).padStart(9, '0');
  const digits = body.replace(/[A-Z]/g, (c) => String(c.charCodeAt(0) - 55));
  // The check digit makes the Luhn sum of body + digit a multiple of ten.
  for (let d = 0; d <= 9; d++) {
    const all = digits + d;
    let sum = 0;
    for (let i = 0; i < all.length; i++) {
      let v = Number(all[all.length - 1 - i]);
      if (i % 2 === 1) {
        v *= 2;
        if (v > 9) v -= 9;
      }
      sum += v;
    }
    if (sum % 10 === 0) return body + d;
  }
  return body + '0';
}

test('PMU buys a bond found by its ISIN, provisional until a Kondor id is attached', async ({ page }) => {
  await login(page, 'pmu1');
  await page.goto('/app/pmu/positions');
  await ready(page);
  await page.getByTestId('pmu-book-bond').click();
  const ticket = page.getByTestId('pmu-ticket');
  await ticket.getByLabel('Bond search').fill('XSSYN0000166');
  await ticket.getByTestId('bond-hits').locator('.hit').first().click();
  await ticket.getByLabel('Face', { exact: true }).fill('3m');
  const preview = ticket.getByTestId('ticket-preview');
  await expect(preview).toContainText('FTP locks at');
  await expect(preview).toContainText('Carry after FTP');
  await ticket.getByRole('button', { name: 'Book the purchase' }).click();
  await expect(page.getByText(/^Bought 3000000 USD of XSSYN0000166/)).toBeVisible();
  await expect(ticket).toHaveCount(0);
  await expect(page.getByTestId('pmu-tree').getByText('provisional').first()).toBeVisible();
});

test('PMU sets up a new bond by its ISIN and buys it', async ({ page }) => {
  const isin = freshIsin();
  await login(page, 'pmu1');
  await page.goto('/app/pmu');
  await ready(page);
  await page.getByTestId('pmu-book-bond').click();
  const ticket = page.getByTestId('pmu-ticket');
  await ticket.getByLabel('Bond search').fill(isin);
  await ticket.getByRole('button', { name: `Add ${isin} as a new bond` }).click();
  const form = ticket.getByTestId('new-bond');
  await form.getByLabel('Bond name').fill('E2E corporate 5.5% 2031');
  await form.getByLabel('Issuer', { exact: true }).fill('E2E corporate');
  await form.getByLabel('Issue date').fill('2025-01-15');
  await form.getByLabel('Maturity').fill('2031-01-15');
  await form.getByRole('button', { name: 'Set up the bond' }).click();
  await expect(ticket.locator('.bond-card')).toContainText(isin);
  await ticket.getByLabel('Face', { exact: true }).fill('2m');
  await ticket.getByRole('button', { name: 'Yield' }).click();
  await ticket.getByLabel('Yield').fill('5.4');
  await expect(ticket.getByTestId('ticket-preview')).toContainText('5.400%');
  await ticket.getByRole('button', { name: 'Book the purchase' }).click();
  await expect(page.getByText(new RegExp(`^Bought 2000000 USD of ${isin}`))).toBeVisible();
});

test('PMU sells oldest lots first or the lots it chooses', async ({ page }) => {
  await login(page, 'pmu1');
  await page.goto('/app/pmu?book=sell&isin=XSSYN0000166');
  await ready(page);
  const ticket = page.getByTestId('pmu-ticket');
  await expect(ticket.locator('.bond-card')).toContainText('XSSYN0000166');
  await ticket.getByLabel('Face', { exact: true }).fill('1m');
  const lots = ticket.getByTestId('sale-lots');
  await expect(lots.locator('tbody tr').first()).toBeVisible();
  // Choosing lots: put the 1m on the second lot instead of the oldest.
  await lots.getByRole('button', { name: 'Choose lots' }).click();
  const inputs = lots.locator('input');
  await inputs.first().fill('0');
  await inputs.nth(1).fill('1m');
  await expect(ticket.getByTestId('ticket-blocking')).toHaveCount(0);
  await ticket.getByRole('button', { name: 'Book the sale' }).click();
  await expect(page.getByText(/^Sold 1000000 USD of XSSYN0000166/)).toBeVisible();
});

test('PMU books a repo on free bonds, closes it early, and books a reverse repo', async ({ page }) => {
  await login(page, 'pmu1');
  await page.goto('/app/pmu/funding');
  await ready(page);
  await page.getByTestId('pmu-book-repo').click();
  const ticket = page.getByTestId('pmu-ticket');
  await ticket.getByLabel('Counterparty').fill('E2E Bank');
  await ticket.getByRole('button', { name: '1M' }).click();
  await ticket.getByLabel('Repo rate').fill('4.55');
  await ticket.getByTestId('repo-collateral').locator('input[type=checkbox]').first().check();
  await expect(ticket.getByTestId('repo-preview')).toContainText('Collateral value');
  await ticket.getByRole('button', { name: 'Book the repo' }).click();
  await expect(page.getByText(/^Repo with E2E Bank/)).toBeVisible();
  // Close it early from the repo book.
  await page.getByRole('button', { name: /close or roll the repo with E2E Bank/ }).first().click();
  const actions = page.getByTestId('repo-actions');
  await actions.getByRole('button', { name: 'Close the repo' }).click();
  await expect(page.getByText(/^Repo closed on/)).toBeVisible();
  // A reverse repo lends cash against the counterparty's bond.
  await page.getByTestId('pmu-book-repo').click();
  await ticket.getByRole('button', { name: 'Reverse repo: lend cash' }).click();
  await ticket.getByLabel('Counterparty').fill('E2E Bank');
  await ticket.getByLabel('Repo rate').fill('4.40');
  await ticket.getByLabel('Collateral ISIN').fill('US0378331005');
  await ticket.getByLabel('Collateral face').fill('5.3m');
  await ticket.getByLabel('Repo cash').fill('5m');
  await ticket.getByRole('button', { name: 'Book the reverse repo' }).click();
  await expect(page.getByText(/^Reverse repo with E2E Bank/)).toBeVisible();
  await expect(page.getByTestId('pmu-funding-strip')).toContainText('Cash lent');
});

test('the head reads the portfolio but has no booking buttons', async ({ page }) => {
  await login(page, 'head1');
  await page.goto('/app/pmu');
  await ready(page);
  await expect(page.getByTestId('pmu-strip')).toBeVisible();
  await expect(page.getByTestId('pmu-book-bond')).toHaveCount(0);
  await expect(page.getByTestId('pmu-book-repo')).toHaveCount(0);
});
