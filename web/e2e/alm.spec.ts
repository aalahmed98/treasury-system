import { expect, test } from '@playwright/test';
import { login, ready } from './helpers';

const TOOLS = ['lcr', 'nsfr', 'gap', 'nii', 'eve', 'stress', 'attribution', 'ird', 'funding'];

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
  await page.getByTestId('shock-bar').getByLabel('Rate shock').selectOption('PARALLEL_UP');
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
  const mode = page.getByTestId('balance-sheet-mode');
  await mode.getByRole('button', { name: 'Run-off', exact: true }).click();
  const runoff = await results(page);
  expect(runoff).not.toBe(dflt);
  await expect(page.locator('.alm-results')).toContainText(/balancing/i);
  await expect(page.getByTestId('nii-modes').locator('tr.marked')).toContainText('Run-off');
  // Dynamic with no growth set is the constant balance sheet; growth in assets changes it.
  await mode.getByRole('button', { name: 'Dynamic', exact: true }).click();
  const flat = await results(page);
  expect(flat).not.toBe(runoff);
  await page.locator('.arow', { hasText: /^Assets/ }).locator('input').fill('10');
  const grown = await results(page);
  expect(grown).not.toBe(flat);
  await expect(page.getByTestId('nii-modes').locator('tr.marked')).toContainText('Dynamic');
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

test('one rate shock is chosen once and every rates tab runs under it', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/gap');
  await ready(page);
  const bar = page.getByTestId('shock-bar');
  const badge = page.getByTestId('shock-badge');
  const strip = page.getByTestId('alm-strip-bar');
  const open = async (name: string) => {
    await strip.getByRole('button', { name, exact: true }).click();
    await ready(page);
  };
  // On today's curve the gap says what a shock would add, and nothing is marked as simulated.
  await expect(page.getByTestId('gap-effect')).toContainText('Choose one above');
  await expect(badge).toHaveCount(0);
  await expect(bar).not.toHaveAttribute('data-on', '1');

  // A custom shock starts as +100 bp on every tenor and adds the income effect to the gap.
  await bar.getByLabel('Rate shock').selectOption('CUSTOM');
  await expect(badge).toContainText('Custom parallel +100 bp');
  await expect(bar).toHaveAttribute('data-on', '1');
  await expect(page.getByTestId('gap-buckets')).toContainText('Effect on 12m NII');
  const everyTenor = await page.getByTestId('gap-effect').innerText();

  // Aimed at the tenors up to a year, it is a different shock with a different effect.
  await bar.getByRole('button', { name: /^Tenors/ }).click();
  await page.getByRole('button', { name: 'Over 1Y', exact: true }).click();
  await expect(badge).toContainText('Custom +100 bp at 2Y to 10Y+');
  await expect(page.getByTestId('gap-effect')).not.toHaveText(everyTenor);
  await page.getByTestId('shock-buckets').getByRole('button', { name: '1Y', exact: true }).click();
  await expect(badge).toContainText('Custom +100 bp at 1Y to 10Y+');

  // The same shock follows to EVE, NII and the stress table without being set again.
  await open('EVE');
  await expect(badge).toContainText('Custom +100 bp at 1Y to 10Y+');
  await expect(page.getByTestId('duration-gap')).toContainText('Duration gap');
  await expect(page.locator('.alm-results')).toContainText('The formula assumes every tenor moves by the same amount');
  await open('NII / NIM');
  await expect(badge).toContainText('Custom +100 bp at 1Y to 10Y+');
  await open('Stress');
  await expect(page.getByTestId('rate-scenarios').locator('tr.marked')).toContainText('Custom +100 bp at 1Y to 10Y+');

  // Liquidity tools keep the bar where it was and say the shock does not reach them; the
  // dashboard shows official figures and has no bar at all.
  await open('LCR');
  await expect(page.getByTestId('shock-note')).toHaveText('no effect on LCR');
  await expect(badge).toHaveCount(0);
  await strip.getByRole('button', { name: 'Dashboard', exact: true }).click();
  await expect(bar).toHaveCount(0);

  // It survives a reload of the tab, and Reset returns to today's curve.
  await page.goto('/app/alm/eve');
  await ready(page);
  await expect(badge).toContainText('Custom +100 bp at 1Y to 10Y+');
  await bar.getByRole('button', { name: /^Tenors/ }).click();
  await page.getByRole('button', { name: 'All tenors', exact: true }).click();
  await expect(page.getByTestId('duration-gap')).toContainText('Estimate from the formula');
  await bar.getByRole('button', { name: 'Reset', exact: true }).click();
  await expect(badge).toHaveCount(0);
  await expect(page.getByTestId('duration-gap')).not.toContainText('Estimate from the formula');
});

test('deposit behaviour: the core share and its decay move the gap and EVE', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/gap');
  await ready(page);
  const before = await results(page);
  const grid = page.getByTestId('deposit-behaviour');
  // Current accounts: life, core share, decay. Half core puts the other half overnight.
  await grid.locator('input').nth(1).fill('50');
  const half = await results(page);
  expect(half).not.toBe(before);
  await grid.locator('input').nth(2).fill('25');
  const decayed = await results(page);
  expect(decayed).not.toBe(half);
  // The same what-if carries to EVE, where the assumptions show it too.
  await page.getByTestId('alm-strip-bar').getByRole('button', { name: 'EVE', exact: true }).click();
  await ready(page);
  await expect(page.getByTestId('deposit-behaviour').locator('input').nth(1)).toHaveValue('50');
  await expect(page.getByText(/what-if · 2 changes/)).toBeVisible();
});

test('trial hedges: added once, they change every rates tool and no liquidity tool', async ({ page }) => {
  await login(page, 'alm1');
  await page.goto('/app/alm/eve');
  await ready(page);
  const bar = page.getByTestId('shock-bar');
  const strip = page.getByTestId('alm-strip-bar');
  const hedges = page.getByTestId('hedges-button');
  const effect = page.getByTestId('hedge-effect');
  const badge = page.getByTestId('hedge-badge');
  const open = async (name: string) => {
    await strip.getByRole('button', { name, exact: true }).click();
    await ready(page);
  };
  await bar.getByLabel('Rate shock').selectOption('PARALLEL_UP');
  await expect(page.getByTestId('shock-badge')).toContainText('Parallel up');
  const unhedged = await page.getByTestId('headline').innerText();
  await expect(hedges).toContainText('none');
  await expect(effect).toHaveCount(0);

  // The form's defaults are a pay-fixed swap at the market rate: it gains when rates rise.
  await hedges.click();
  const panel = page.getByRole('dialog', { name: 'Trial hedges' });
  await expect(panel).toContainText(/at market \(\d+\.\d\d%\)/);
  await panel.getByRole('button', { name: 'Add hedge' }).click();
  await expect(page.getByTestId('hedge-list')).toContainText('Pay-fixed swap');
  await expect(hedges).toContainText('1 trial hedge');
  await expect(badge).toContainText('with 1 trial hedge');
  await expect(effect).toContainText('Without the trial hedges the change in EVE would be');
  await expect(page.getByTestId('headline')).not.toHaveText(unhedged);

  // The same hedge on the other rates tools, each saying what it changed.
  await open('Gap');
  await expect(effect).toContainText('cumulative gap at one year');
  await expect(badge).toBeVisible();
  await open('NII / NIM');
  await expect(effect).toContainText('The trial hedges change NII by');
  await open('Stress');
  await expect(effect).toContainText('worst EVE loss');

  // Liquidity tools do not see it.
  await open('LCR');
  await expect(badge).toHaveCount(0);
  await expect(page.getByTestId('shock-note')).toHaveText('no effect on LCR');

  // It survives a reload of the tab and goes when removed.
  await page.goto('/app/alm/eve');
  await ready(page);
  await expect(hedges).toContainText('1 trial hedge');
  await hedges.click();
  await page.getByRole('button', { name: 'Remove all' }).click();
  await expect(hedges).toContainText('none');
  await expect(effect).toHaveCount(0);
  await expect(badge).toHaveCount(0);
});
