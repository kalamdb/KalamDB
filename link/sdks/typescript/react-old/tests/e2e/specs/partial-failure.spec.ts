import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "partfail";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 3);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("B5: good query still renders rows when sibling fails", async ({ page }) => {
  await page.goto(`/?page=partial-failure&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("good-count")).toHaveText("3");
  await expect(page.getByTestId("good-status")).toHaveText("live");
  await expect(page.getByTestId("bad-status")).toHaveText("error");
  await expect(page.getByTestId("bad-error")).not.toHaveText("");
});
