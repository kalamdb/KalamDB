import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "remount";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 2);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("WASM bug repro: remount does not duplicate rows (requires WASM fix #270)", async ({ page }) => {
  await page.goto(`/?page=remount&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("2");

  await page.getByTestId("remount").click();
  await expect(page.getByTestId("row-count")).toHaveText("2");

  await page.getByTestId("remount").click();
  await expect(page.getByTestId("row-count")).toHaveText("2");
});

test("toggle mount/unmount cleanly disposes subscriptions", async ({ page }) => {
  await page.goto(`/?page=remount&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("2");

  await page.getByTestId("toggle-mount").click();
  await expect(page.getByTestId("unmounted")).toBeVisible();

  await page.getByTestId("toggle-mount").click();
  await expect(page.getByTestId("row-count")).toHaveText("2");
});
