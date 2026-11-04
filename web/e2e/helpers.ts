import { expect, type Page } from '@playwright/test';

export async function login(page: Page, user: string, password = 'password') {
  await page.goto('/app/');
  await page.getByLabel('Username').fill(user);
  await page.getByLabel('Password').fill(password);
  // The server rate-limits sign-ins per address; a long test run can trip it, so wait it out.
  for (let attempt = 0; ; attempt++) {
    await page.getByRole('button', { name: 'Sign in' }).click();
    const outcome = await Promise.race([
      page.locator('header.topbar').waitFor({ timeout: 15_000 }).then(() => 'ok' as const),
      page.locator('.notice.bad').waitFor({ timeout: 15_000 }).then(() => 'error' as const),
    ]);
    if (outcome === 'ok') return;
    const text = (await page.locator('.notice.bad').textContent()) ?? '';
    if (!/too many attempts/i.test(text) || attempt >= 4) throw new Error(`sign-in failed: ${text}`);
    const secs = Number(/in (\d+) second/.exec(text)?.[1] ?? 6);
    await page.waitForTimeout((secs + 1) * 1000);
  }
}

export async function setTheme(page: Page, theme: 'light' | 'dark') {
  await page.addInitScript((t) => {
    localStorage.setItem('nbb-theme', t);
  }, theme);
}

export async function ready(page: Page) {
  await page.locator('main [data-ready="1"]').first().waitFor({ timeout: 30_000 });
  await page.waitForTimeout(600);
}

/** The page scrolls inside <main>, so a full-page screenshot grows the viewport to its content. */
export async function fullPage(page: Page, path: string) {
  const h = await page.evaluate(() => {
    const m = document.querySelector('main');
    return m ? m.scrollHeight + 44 + 26 + 2 : 900;
  });
  await page.setViewportSize({ width: 1440, height: Math.min(4000, Math.max(900, h)) });
  await page.waitForTimeout(250);
  await page.screenshot({ path });
  await page.setViewportSize({ width: 1440, height: 900 });
}
