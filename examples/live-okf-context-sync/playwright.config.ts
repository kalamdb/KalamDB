import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'tests',
  testMatch: 'e2e.spec.mjs',
  timeout: 30_000,
  use: {
    baseURL: process.env.OKF_SYNC_APP_URL ?? 'http://127.0.0.1:5177',
  },
});
