import { expect, test } from "@playwright/test";
import {
  clickNav,
  openFreshSqlTab,
  openSqlStudio,
  runSql,
  skipUnlessLiveAdminUi,
  uniqueName,
} from "./helpers";

test.describe.configure({ timeout: 180_000 });

test("SQL Studio creates a user and the Users page reflects later ALTER USER", async ({ page }) => {
  await skipUnlessLiveAdminUi(page);

  const username = uniqueName("pwe2e_user");
  const email = `${username}@example.test`;
  const password = "PlaywrightE2e!1";

  try {
    await openSqlStudio(page);

    await test.step("CREATE USER from SQL Studio", async () => {
      await runSql(page, `
CREATE USER '${username}' WITH PASSWORD '${password}' ROLE 'user' EMAIL '${email}';
`.trim());
      await page.getByRole("tab", { name: /^Log/ }).click();
      await expect(page.getByText(/created|USER/i).first()).toBeVisible({ timeout: 10_000 });
    });

    await test.step("Users page lists the SQL-created account", async () => {
      await clickNav(page, "/ui/users");
      await expect(page.getByRole("heading", { name: "Users", level: 1 })).toBeVisible({ timeout: 30_000 });
      await expect(page.getByPlaceholder("Search users...")).toBeVisible();
      await page.getByPlaceholder("Search users...").fill(username);
      const row = page.getByRole("row").filter({ hasText: username });
      await expect(row).toBeVisible({ timeout: 30_000 });
      await expect(row).toContainText(email);
      await expect(row).toContainText("user");
    });

    await test.step("ALTER USER in SQL Studio then refresh Users", async () => {
      await openFreshSqlTab(page);
      await runSql(page, `ALTER USER '${username}' SET ROLE dba;`);
      await page.getByRole("tab", { name: /^Log/ }).click();
      await expect(page.getByText(/ALTER|updated|role|dba/i).first()).toBeVisible({ timeout: 10_000 });

      await clickNav(page, "/ui/users");
      await expect(page.getByPlaceholder("Search users...")).toBeVisible({ timeout: 30_000 });
      await page.getByPlaceholder("Search users...").fill(username);
      await page.getByRole("button", { name: "Refresh users" }).click();
      const row = page.getByRole("row").filter({ hasText: username });
      await expect(row).toBeVisible({ timeout: 30_000 });
      await expect(row).toContainText("dba");
      await expect(row).toContainText(email);
    });
  } finally {
    await openSqlStudio(page).catch(() => undefined);
    await runSql(page, `DROP USER IF EXISTS '${username}';`).catch(() => undefined);
  }
});
