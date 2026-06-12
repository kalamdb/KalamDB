import { defineConfig } from '@playwright/test';

const devPort = 4173;
const kalamdbUrl = process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';

export default defineConfig({
  testDir: './tests',
  globalSetup: './tests/global-setup.ts',
  timeout: 60_000,
  fullyParallel: false,
  use: {
    baseURL: `http://127.0.0.1:${devPort}`,
    headless: true,
  },
  webServer: {
    command: `npm run dev -- --host 127.0.0.1 --port ${devPort} --strictPort`,
    port: devPort,
    env: {
      ...process.env,
      VITE_KALAMDB_URL: kalamdbUrl,
      VITE_KALAMDB_USER: process.env.VITE_KALAMDB_USER ?? 'demo-user',
      VITE_KALAMDB_PASSWORD: process.env.VITE_KALAMDB_PASSWORD ?? 'demo123',
    },
    reuseExistingServer: false,
    timeout: 120_000,
  },
});