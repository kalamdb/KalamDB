import { expect, test } from "@playwright/test";
import {
  clickNav,
  openSqlStudio,
  runSql,
  skipUnlessLiveAdminUi,
  uniqueName,
} from "./helpers";

test.describe.configure({ timeout: 180_000 });

test("SQL Studio creates a topic and the Topics UI shows the new chrome", async ({ page }) => {
  await skipUnlessLiveAdminUi(page);

  const ns = uniqueName("pwe2e_topic");
  const topicId = `${ns}.events`;

  try {
    await openSqlStudio(page);

    await test.step("create a namespaced topic from SQL Studio", async () => {
      await runSql(page, `
CREATE NAMESPACE ${ns};
CREATE TOPIC ${topicId} PARTITIONS 2;
`.trim());
      await page.getByRole("tab", { name: /^Log/ }).click();
      await expect(page.getByText(/created|TOPIC/i).first()).toBeVisible({ timeout: 10_000 });

      await runSql(page, `SELECT topic_id, partitions FROM system.topics WHERE topic_id = '${topicId}';`);
      await page.getByRole("tab", { name: /^Results/ }).click();
      await expect(page.getByTestId("results-column-header-topic_id")).toBeVisible({ timeout: 15_000 });
      await expect(page.locator('[data-column-name="topic_id"]').first()).toContainText(topicId);
      await expect(page.locator('[data-column-name="partitions"]').first()).toContainText("2");
    });

    await test.step("Topics list and Inspect page pick up the SQL change", async () => {
      await clickNav(page, "/ui/streaming/topics");
      await expect(page.getByRole("heading", { name: "Streaming" })).toBeVisible({ timeout: 30_000 });
      await expect(page.getByPlaceholder("Search topics...")).toBeVisible();

      await page.getByRole("button", { name: "Refresh" }).click();
      await page.getByPlaceholder("Search topics...").fill(topicId);
      const row = page.getByRole("row").filter({ hasText: topicId });
      await expect(row).toBeVisible({ timeout: 30_000 });
      await expect(row).toContainText("2");
      await row.getByRole("button", { name: "Inspect" }).click();

      await expect(page.getByRole("heading", { name: topicId })).toBeVisible({ timeout: 20_000 });
      const crumb = page.getByRole("navigation", { name: "breadcrumb" });
      await expect(crumb.getByText("Streaming")).toBeVisible();
      await expect(crumb.getByRole("link", { name: "Topics" })).toBeVisible();
      await expect(crumb.getByText(topicId)).toBeVisible();
      await expect(page.getByText("Topic Summary")).toBeVisible();
      await expect(page.getByText("Partitions", { exact: true }).locator("xpath=following-sibling::p")).toHaveText("2");

      const inspectTab = page.getByRole("tab", { name: "Inspect Messages" });
      const offsetsTab = page.getByRole("tab", { name: "Committed Offsets" });
      const sqlTab = page.getByRole("tab", { name: "SQL Studio Shortcuts" });
      await expect(inspectTab).toBeVisible();
      await expect(inspectTab).toHaveAttribute("aria-selected", "true");
      await expect(offsetsTab).toBeVisible();
      await expect(sqlTab).toBeVisible();
      await expect(page.getByRole("tab", { name: "Consumers" })).toHaveCount(0);
      await expect(page.getByText("Message Inspector Controls")).toBeVisible();

      await offsetsTab.click();
      await expect(offsetsTab).toHaveAttribute("aria-selected", "true");
      await expect(page.getByRole("tabpanel", { name: "Committed Offsets" })).toBeVisible();
      await expect(page.getByText("No committed offsets for this topic yet.")).toBeVisible();

      await sqlTab.click();
      await expect(sqlTab).toHaveAttribute("aria-selected", "true");
      await expect(page.getByRole("button", { name: "Open Topic SQL" })).toBeVisible();
      await expect(page.getByText(`SELECT * FROM system.topics WHERE topic_id = '${topicId}';`)).toBeVisible();

      await crumb.getByRole("link", { name: "Topics" }).click();
      await expect(page.getByPlaceholder("Search topics...")).toBeVisible({ timeout: 15_000 });
      await expect(page.getByRole("row").filter({ hasText: topicId })).toBeVisible();
    });
  } finally {
    await openSqlStudio(page).catch(() => undefined);
    await runSql(page, `DROP NAMESPACE IF EXISTS ${ns} CASCADE;`).catch(() => undefined);
  }
});
