import { defineConfig } from '@playwright/test';

// Runs on the Edge already installed on the desk: no browser download. The server under test is the
// one in WSL (or wherever TREASURY_URL points).
export default defineConfig({
  testDir: 'e2e',
  timeout: 90_000,
  retries: 0,
  workers: 1,
  reporter: 'list',
  use: {
    baseURL: process.env.TREASURY_URL ?? 'http://localhost:8787',
    channel: 'msedge',
    headless: true,
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    ignoreHTTPSErrors: true,
  },
});
