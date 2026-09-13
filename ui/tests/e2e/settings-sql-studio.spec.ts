import { expect, test } from "@playwright/test";
import {
  clickNav,
  openSqlStudio,
  readSqlRows,
  runSql,
  skipUnlessLiveAdminUi,
} from "./helpers";

test.describe.configure({ timeout: 180_000 });

const SETTING_NAMES = [
  "cluster.cluster_id",
  "server.host",
  "topics.visibility_timeout_secs",
] as const;

test("SQL Studio reads system.settings and the Settings pages show the same values", async ({ page }) => {
  await skipUnlessLiveAdminUi(page);

  const quotedNames = SETTING_NAMES.map((name) => `'${name}'`).join(", ");
  let settings: Record<string, string> = {};

  await openSqlStudio(page);

  await test.step("SELECT live settings from SQL Studio", async () => {
    await runSql(page, `
SELECT name, value
FROM system.settings
WHERE name IN (${quotedNames})
ORDER BY name;
`.trim());
    await page.getByRole("tab", { name: /^Results/ }).click();
    const rows = await readSqlRows(page, ["name", "value"], SETTING_NAMES.length);
    settings = Object.fromEntries(rows.map((row) => [row.name, row.value]));
    for (const name of SETTING_NAMES) {
      expect(settings[name], `missing SQL setting ${name}`).toBeTruthy();
    }
  });

  await test.step("Settings All page shows the SQL values and Current User", async () => {
    await clickNav(page, "/ui/settings");
    await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText("Current User")).toBeVisible();
    await expect(page.getByText("root", { exact: true }).first()).toBeVisible();

    const sections = page.getByRole("navigation", { name: "Settings sections" });
    await expect(sections.getByRole("link", { name: "All Settings" })).toBeVisible();
    await expect(sections.getByRole("link", { name: "Cluster" })).toBeVisible();
    await expect(sections.getByRole("link", { name: "Storages" })).toBeVisible();
    await expect(sections.getByRole("link", { name: "Security" })).toBeVisible();

    for (const name of SETTING_NAMES) {
      const heading = page.getByRole("heading", { name, exact: true });
      await heading.scrollIntoViewIfNeeded();
      await expect(heading).toBeVisible();
      const value = heading.locator("xpath=ancestor::div[contains(@class,'justify-between')]").locator("code");
      await expect(value).toContainText(settings[name]);
    }
  });

  await test.step("Cluster, Storages, and Security settings sections still load", async () => {
    const sections = page.getByRole("navigation", { name: "Settings sections" });

    await sections.getByRole("link", { name: "Cluster" }).click();
    await expect(page.getByText("Cluster Health")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByText("Cluster Nodes")).toBeVisible();

    await sections.getByRole("link", { name: "Storages" }).click();
    await expect(page.getByRole("button", { name: "Create Storage" })).toBeVisible({ timeout: 20_000 });

    await sections.getByRole("link", { name: "Security" }).click();
    await expect(page.getByText("Security Settings", { exact: true })).toBeVisible({ timeout: 15_000 });
    await expect(page.getByText("No settings available yet.")).toBeVisible();

    await sections.getByRole("link", { name: "All Settings" }).click();
    await expect(page.getByRole("heading", { name: "cluster.cluster_id", exact: true })).toBeVisible({
      timeout: 20_000,
    });
  });
});
