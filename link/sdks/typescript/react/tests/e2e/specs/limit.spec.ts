import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "limit";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 5);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("LiveQuery respects limit", async ({ page }) => {
  await page.goto(`/?page=limit&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("status")).toHaveText("live");
  await expect(page.getByTestId("row-count")).toHaveText("3");
});

test("insert pushes the limit window forward (oldest drops out)", async ({ page }) => {
  await page.goto(`/?page=limit&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("3");

  await page.getByTestId("add").click();
  await page.getByTestId("add").click();

  await expect(page.getByTestId("row-count")).toHaveText("3");
});
