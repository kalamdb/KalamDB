import { defineConfig } from '@playwright/test';

const kalamdbUrl = process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';

export default defineConfig({
  testDir: './tests',
  timeout: 60_000,
  fullyParallel: false,
  use: {
    baseURL: 'http://127.0.0.1:4173',
    headless: true,
  },
  webServer: {
    command: 'npm run dev -- --host 127.0.0.1 --port 4173 --strictPort',
    env: {
      ...process.env,
      VITE_KALAMDB_URL: kalamdbUrl,
      VITE_KALAMDB_USER: process.env.VITE_KALAMDB_USER ?? 'demo-user',
      VITE_KALAMDB_PASSWORD: process.env.VITE_KALAMDB_PASSWORD ?? 'demo123',
    },
    port: 4173,
    reuseExistingServer: false,
    timeout: 60_000,
  },
});