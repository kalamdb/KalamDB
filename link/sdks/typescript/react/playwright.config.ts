import { defineConfig, devices } from "@playwright/test";

const KALAM_URL = process.env.KALAM_URL ?? "http://127.0.0.1:2900";

export default defineConfig({
  testDir: "./tests/e2e/specs",
  fullyParallel: false,
  workers: 1,
  reporter: process.env.CI ? "github" : "list",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: "http://127.0.0.1:5181",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "npm --prefix tests/e2e/app run dev",
    url: "http://127.0.0.1:5181",
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
    env: {
      VITE_KALAMDB_URL: KALAM_URL,
      VITE_KALAM_USER: process.env.KALAM_USER ?? "root",
      VITE_KALAM_PASSWORD: process.env.KALAM_PASSWORD ?? "kalamdb123",
    },
  },
});
