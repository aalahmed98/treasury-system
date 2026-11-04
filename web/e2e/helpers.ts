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
