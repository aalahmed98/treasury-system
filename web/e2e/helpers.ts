import { expect, type Page } from '@playwright/test';

export async function login(page: Page, user: string, password = 'password') {
  await page.goto('/app/');
  await page.getByLabel('Username').fill(user);
  await page.getByLabel('Password').fill(password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.locator('header.topbar')).toBeVisible();
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
