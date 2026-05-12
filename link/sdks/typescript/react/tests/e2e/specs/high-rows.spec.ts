import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages } from "../helpers/schema-setup";

const SUFFIX = "highrows";
const ROW_COUNT = 75; // above KalamDB's silent 50-row default cap, below rate limit threshold for sequential inserts

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "stress", ROW_COUNT);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("LiveQuery with explicit large limit renders ROW_COUNT rows", async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto(`/?page=high-rows&schema=${SUFFIX}&room=stress`);
  await expect(page.getByTestId("status")).toHaveText("live", { timeout: 30_000 });
  await expect(page.getByTestId("row-count")).toHaveText(String(ROW_COUNT), { timeout: 30_000 });
});
