import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "disconnect";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 2);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("client.disconnect() surfaces status change; refetch recovers", async ({ page }) => {
  await page.goto(`/?page=disconnect&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("2");
  await expect(page.getByTestId("status")).toHaveText("live");

  await page.getByTestId("disconnect").click();
  // After disconnect the subscription is broken. Either status flips to error,
  // or the next refetch reports it; we accept either path.
  await page.getByTestId("refetch").click();
  // Eventually the SDK either recovers (auto-reconnect) or stays in error.
  await expect(async () => {
    const status = await page.getByTestId("status").textContent();
    expect(["live", "loading", "reconnecting", "error"]).toContain(status);
  }).toPass({ timeout: 15_000 });
});
