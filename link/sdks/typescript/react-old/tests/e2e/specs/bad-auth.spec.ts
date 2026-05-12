import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema } from "../helpers/schema-setup";

const SUFFIX = "badauth";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("LiveQuery with bad credentials surfaces error and stays unconnected", async ({ page }) => {
  await page.goto(`/?page=bad-auth&schema=${SUFFIX}`);
  await expect(page.getByTestId("status")).toHaveText("error", { timeout: 15_000 });
  await expect(page.getByTestId("row-count")).toHaveText("0");
  await expect(page.getByTestId("error")).not.toHaveText("");
});
