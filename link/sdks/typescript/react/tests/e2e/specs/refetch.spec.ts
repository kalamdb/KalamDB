import { test, expect } from "@playwright/test";
import { setupSchema, teardownSchema, seedMessages, insertMessage } from "../helpers/schema-setup";

const SUFFIX = "refetch";

test.beforeAll(async () => {
  await setupSchema(SUFFIX);
  await seedMessages(SUFFIX, "main", 2);
});
test.afterAll(async () => {
  await teardownSchema(SUFFIX);
});

test("refetch re-establishes the subscription and stays consistent", async ({ page }) => {
  await page.goto(`/?page=refetch&schema=${SUFFIX}&room=main`);
  await expect(page.getByTestId("row-count")).toHaveText("2");

  // Insert from the server side (bypasses the React client to bait a refetch use-case)
  await insertMessage(SUFFIX, "main", "out-of-band");
  // Live subscription already covers it; row count moves to 3 either way
  await expect(page.getByTestId("row-count")).toHaveText("3");

  await page.getByTestId("refetch").click();
  await expect(page.getByTestId("status")).toHaveText("live");
  await expect(page.getByTestId("row-count")).toHaveText("3");
});

test("refetch during a fresh mutation does not lose rows", async ({ page }) => {
  await page.goto(`/?page=refetch&schema=${SUFFIX}&room=other`);
  await expect(page.getByTestId("status")).toHaveText("live");
  await expect(page.getByTestId("row-count")).toHaveText("0");

  // First insert — wait for it to land in the subscription before refetching,
  // otherwise refetch() resets the controller mid-flight and the pending
  // mutation's snapshot delivery is what we're actually testing here.
  await page.getByTestId("add").click();
  await expect(page.getByTestId("row-count")).toHaveText("1");

  await page.getByTestId("refetch").click();
  await expect(page.getByTestId("row-count")).toHaveText("1");

  await page.getByTestId("add").click();
  await expect(page.getByTestId("row-count")).toHaveText("2");
  await expect(page.getByTestId("status")).toHaveText("live");
});
