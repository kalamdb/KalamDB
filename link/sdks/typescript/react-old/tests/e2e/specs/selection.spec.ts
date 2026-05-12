import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "selection";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 2);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("useLiveSelection derives view from rows", async ({ page }) => {
  await page.goto(`/?page=selection&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("total")).toHaveText("2");
  await expect(page.getByTestId("body")).toHaveCount(2);

  await page.getByTestId("add").click();
  await expect(page.getByTestId("total")).toHaveText("3");
});
