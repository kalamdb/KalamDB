import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.KALAMDB_UI_PLAYWRIGHT_PORT ?? 4175);
const host = "127.0.0.1";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: true,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: `http://${host}:${port}`,
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: `npm run dev -- --host ${host} --port ${port}`,
    url: `http://${host}:${port}/ui/login`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});