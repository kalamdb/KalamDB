import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedComposite } from "../helpers/schema-setup";

const SUFFIX = "compkey";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedComposite(SUFFIX, "main", 3);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("LiveQuery with composite getKey renders rows keyed by (room_id, message_id)", async ({ page }) => {
  await page.goto(`/?page=composite-key&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("status")).toHaveText("live");
  await expect(page.getByTestId("row-count")).toHaveText("3");
  await expect(page.getByTestId("row").first()).toHaveAttribute("data-msg", "msg-0");
  await expect(page.getByTestId("row").last()).toHaveAttribute("data-msg", "msg-2");
});
