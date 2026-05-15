import { defineConfig, devices } from "@playwright/test";

// The tests assume the dev stack is up: KalamDB at :8080, the backend +
// agent + vite running. `npm run dev` boots all three. The tests do NOT
// auto-start the stack — the user is expected to run `npm run setup`
// followed by `npm run dev` in a separate terminal, then `npm test`.
//
// Tests force LLM_PROVIDER=mock via the agent's env so the Stop test is
// deterministic (the mock adapter responds to "__slow_stream__").

export default defineConfig({
  testDir: "./tests",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "retain-on-failure",
    actionTimeout: 10_000,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
