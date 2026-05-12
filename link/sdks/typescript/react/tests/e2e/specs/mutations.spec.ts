import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema } from "../helpers/schema-setup";

const SUFFIX = "mutations";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("full CRUD cycle via drizzle mode", async ({ page }) => {
  await page.goto(`/?page=mutations&schema=${SUFFIX}&room=main`);

  await expect(page.getByTestId("status")).toHaveText("live");
  await expect(page.getByTestId("row-count")).toHaveText("0");

  await page.getByTestId("add").click();
  await expect(page.getByTestId("row-count")).toHaveText("1");

  // Small gap so the next createdAt is strictly greater (orderBy is stable).
  await page.waitForTimeout(50);
  await page.getByTestId("add").click();
  await expect(page.getByTestId("row-count")).toHaveText("2");

  const firstRow = page.getByTestId("row").first();
  const originalBody = (await firstRow.getByTestId("row-body").textContent()) ?? "";
  await firstRow.getByTestId("edit").click();
  await expect(firstRow.getByTestId("row-body")).toHaveText(`${originalBody}!`);

  await firstRow.getByTestId("del").click();
  await expect(page.getByTestId("row-count")).toHaveText("1");
});

test("per-row updating set tracks the pending row", async ({ page }) => {
  await page.goto(`/?page=mutations&schema=${SUFFIX}&room=tracking`);
  await expect(page.getByTestId("status")).toHaveText("live");
  await page.getByTestId("add").click();
  await expect(page.getByTestId("row-count")).toHaveText("1");

  const row = page.getByTestId("row").first();
  await row.getByTestId("edit").click();
  // Eventually settles to "no" but at least the row is visible.
  await expect(row.getByTestId("row-updating")).toHaveText("no");
});
