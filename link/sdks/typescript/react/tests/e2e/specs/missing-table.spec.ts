import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema } from "../helpers/schema-setup";

const SUFFIX = "missing";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("LiveQuery against a non-existent table reports error and does not crash", async ({ page }) => {
  await page.goto(`/?page=missing-table&schema=${SUFFIX}`);
  await expect(page.getByTestId("status")).toHaveText("error");
  await expect(page.getByTestId("row-count")).toHaveText("0");
  await expect(page.getByTestId("error")).not.toHaveText("");
});
