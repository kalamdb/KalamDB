import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "select";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 2);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("useLiveQuery select transform returns derived shape", async ({ page }) => {
  await page.goto(`/?page=select-transform&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("loading")).toHaveText("no");
  await expect(page.getByTestId("total")).toHaveText("2");
  await expect(page.getByTestId("first")).toHaveText("seed-body-0");
  await expect(page.getByTestId("last")).toHaveText("seed-body-1");
});
