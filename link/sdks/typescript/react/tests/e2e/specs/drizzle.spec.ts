import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "drizzle";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 2);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("renders seeded rows", async ({ page }) => {
  await page.goto(`/?page=drizzle&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("2");
  await expect(page.getByTestId("status")).toHaveText("live");
});

test("insert appears via live subscription", async ({ page }) => {
  await page.goto(`/?page=drizzle&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("2");
  await page.getByTestId("add").click();
  await expect(page.getByTestId("row-count")).toHaveText("3");
});
