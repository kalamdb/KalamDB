import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema } from "../helpers/schema-setup";

const SUFFIX = "colmap";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("camelCase JS keys are mapped to snake_case columns on insert", async ({ page }) => {
  await page.goto(`/?page=column-mapping&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("status")).toHaveText("live");
  await expect(page.getByTestId("row-count")).toHaveText("0");
  await page.getByTestId("add-with-camel").click();
  await expect(page.getByTestId("row-count")).toHaveText("1");
  await expect(page.getByTestId("row-author")).toHaveText("Inas");
  await expect(page.getByTestId("row-body")).toHaveText("from-camel");
  await expect(page.getByTestId("error")).toHaveText("");
});
