import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "sql";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 3);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("raw SQL renders seeded rows", async ({ page }) => {
  await page.goto(`/?page=sql&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("3");
  await expect(page.getByTestId("status")).toHaveText("live");
});

test("raw SQL insert appears live", async ({ page }) => {
  await page.goto(`/?page=sql&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("3");
  await page.getByTestId("add").click();
  await expect(page.getByTestId("row-count")).toHaveText("4");
});
