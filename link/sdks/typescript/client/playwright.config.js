import { defineConfig } from '@playwright/test';

const port = Number(process.env.KALAMDB_BROWSER_TEST_PORT ?? 41731);

export default defineConfig({
  testDir: './tests/playwright',
  timeout: 90_000,
  workers: 1,
  reporter: 'list',
  use: {
    browserName: 'chromium',
    headless: true,
    baseURL: `http://127.0.0.1:${port}`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'node ./tests/playwright/static-proxy-server.mjs',
    port,
    reuseExistingServer: !process.env.CI,
    env: {
      ...process.env,
      PORT: String(port),
      STATIC_ROOT: process.cwd(),
      BACKEND_URL: process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900',
    },
  },
});