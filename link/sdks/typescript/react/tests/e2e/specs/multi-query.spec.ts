import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages, seedCounters } from "../helpers/schema-setup";

const SUFFIX = "multi";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 2);
  await seedCounters(SUFFIX, 4);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("LiveQueries reports both query counts and aggregate state", async ({ page }) => {
  await page.goto(`/?page=multi&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("messages-count")).toHaveText("2");
  await expect(page.getByTestId("counters-count")).toHaveText("4");
  await expect(page.getByTestId("aggregate-loading")).toHaveText("no");
  await expect(page.getByTestId("aggregate-connected")).toHaveText("yes");
});
