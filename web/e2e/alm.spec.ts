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

/** The results panel once the recompute for the latest inputs has landed. */
async function results(page: import('@playwright/test').Page): Promise<string> {
  await expect(page.locator('.alm-results[data-busy="1"]')).toHaveCount(0);
  await page.waitForTimeout(400); // the what-if is debounced before it is sent
  await expect(page.locator('.alm-results[data-busy="1"]')).toHaveCount(0);
  return page.locator('.alm-results').innerText();
}

test('NII: deposit betas, the default beta and the balance-sheet switch all move the figures', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/nii');
  await ready(page);
  await page.getByRole('button', { name: 'Parallel up', exact: true }).click();
  const before = await results(page);
  // Savings passes on more of the rise: NII falls.
  await page.locator('.arow', { hasText: 'Savings' }).locator('input').fill('80');
  const savings = await results(page);
  expect(savings).not.toBe(before);
  // The default applies to call accounts, which have no entry of their own.
  await page.locator('.arow', { hasText: 'Other products (default)' }).locator('input').fill('90');
  const dflt = await results(page);
  expect(dflt).not.toBe(savings);
  // Running the balance sheet off replaces maturing cohorts with one-month money: a Balancing line.
  await page.getByLabel('Constant balance sheet').uncheck();
  const runoff = await results(page);
  expect(runoff).not.toBe(dflt);
  await expect(page.locator('.alm-results')).toContainText(/balancing/i);
});

test('an assumption with nothing in the snapshot to act on says so', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/nsfr');
  await ready(page);
  await page.getByText('Available stable funding factors').click();
  // Operational deposits have no maturity, so only the first band can apply to them.
  const idle = page.locator('.assumptions .numwrap[data-empty="1"]');
  await expect(idle.first()).toBeVisible();
  await expect(idle.first()).toHaveAttribute('title', /nothing in this snapshot/);
  // Every LCR run-off rate now has balances behind it, and says how much.
  await page.goto('/app/alm/lcr');
  await ready(page);
  await page.getByText('Run-off rates').click();
  const secured = page.locator('.arow', { hasText: 'Secured l1' });
  await expect(secured).toHaveAttribute('title', /applies to/);
  await expect(page.locator('.assumptions .arow[data-empty="1"]')).toHaveCount(0);
});
