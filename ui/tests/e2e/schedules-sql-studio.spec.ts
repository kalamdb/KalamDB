import { expect, type Page, test } from "@playwright/test";
import {
  clickNav,
  openFreshSqlTab,
  openSqlStudio,
  readSqlRows,
  runSql,
  skipUnlessLiveAdminUi,
  uniqueName,
} from "./helpers";

test.describe.configure({ timeout: 180_000 });

function deploySql(ns: string): string {
  return `
CREATE NAMESPACE ${ns};

CREATE TABLE ${ns}.schedule_hits (
  run_id TEXT PRIMARY KEY,
  kind TEXT
) WITH (TYPE = 'SHARED');

CREATE OR REPLACE PROCEDURE ${ns}.tick()
LANGUAGE JAVASCRIPT
COMMENT 'Playwright scheduled tick'
AS $$
  if (ctx.source.kind !== 'schedule' || !ctx.source.scheduleId || !ctx.source.runId
      || typeof ctx.source.scheduledAt !== 'number' || ctx.http !== null) {
    throw new Error('invalid schedule origin');
  }
  return ctx.db.execute(
    "INSERT INTO ${ns}.schedule_hits (run_id, kind) VALUES ('" + ctx.source.runId + "', '" + ctx.source.kind + "')"
  );
$$;

CREATE SCHEDULE ${ns}.clock INTERVAL '1 second' EXECUTE PROCEDURE ${ns}.tick();
`.trim();
}

async function openSchedules(page: Page): Promise<void> {
  await clickNav(page, "/ui/schedules");
  await expect(page.getByRole("heading", { name: "Schedules", level: 1 })).toBeVisible({
    timeout: 30_000,
  });
}

async function refreshSchedules(page: Page): Promise<void> {
  const refresh = page.getByRole("button", { name: "Refresh" });
  await expect(refresh).toBeVisible({ timeout: 15_000 });
  await expect(refresh).toBeEnabled({ timeout: 15_000 });
  await refresh.click();
  await expect(refresh).toBeEnabled({ timeout: 30_000 });
}

test("SQL Studio creates a schedule that runs a procedure and the Schedules UI tracks it", async ({
  page,
}) => {
  await skipUnlessLiveAdminUi(page);

  const ns = uniqueName("pwe2e_sched");
  const scheduleId = `${ns}.clock`;
  const procedureId = `${ns}.tick`;

  try {
    await openSqlStudio(page);

    await test.step("deploy a table, procedure, and 1s interval schedule", async () => {
      await runSql(page, deploySql(ns));
      await page.getByRole("tab", { name: /^Log/ }).click();
      await expect(page.getByText(/created|SCHEDULE|PROCEDURE/i).first()).toBeVisible({
        timeout: 10_000,
      });

      await openFreshSqlTab(page);
      await runSql(
        page,
        `SELECT schedule_id, enabled, interval_ms, routine_id FROM system.schedules WHERE schedule_id = '${scheduleId}';`,
      );
      const rows = await readSqlRows(page, ["schedule_id", "enabled", "interval_ms", "routine_id"], 1);
      expect(rows[0]?.schedule_id).toBe(scheduleId);
      expect(rows[0]?.enabled.toLowerCase()).toBe("true");
      expect(rows[0]?.interval_ms).toBe("1000");
      expect(rows[0]?.routine_id).toBe(procedureId);
    });

    await test.step("Schedules page lists the clock and waits for the procedure to fire", async () => {
      await openSchedules(page);
      await refreshSchedules(page);

      const row = page.getByTestId(`schedules-row-${scheduleId}`);
      await expect(row).toBeVisible({ timeout: 30_000 });
      await expect(row).toContainText(procedureId);
      await expect(row).toContainText("Every 1s");
      await expect(row.getByRole("link", { name: procedureId })).toBeVisible();

      await expect(async () => {
        await refreshSchedules(page);
        await expect(page.getByTestId(`schedules-runs-${scheduleId}`)).toHaveText(/^[1-9]/);
      }).toPass({ timeout: 30_000 });

      await expect(row.getByText("Failed", { exact: true })).toHaveCount(0);
      await expect(row.locator("p.text-destructive")).toHaveCount(0);

      await page.getByRole("button", { name: `View history for ${scheduleId}` }).click();
      await expect(page.getByTestId("schedule-history-table")).toBeVisible();
      await expect(page.getByTestId("schedule-history-table")).toContainText(/Succeeded|ok/i, {
        timeout: 20_000,
      });
      await page.keyboard.press("Escape");
    });

    await test.step("function overview shows the scheduled invocation", async () => {
      await page.getByTestId(`schedules-row-${scheduleId}`).getByRole("link", { name: procedureId }).click();
      await expect(page.getByRole("heading", { name: procedureId })).toBeVisible({ timeout: 20_000 });
      await expect(
        page.getByRole("tabpanel", { name: "Overview" }).getByText("Playwright scheduled tick"),
      ).toBeVisible();
      await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute("data-state", "active");
      await expect(page.getByText("Schedule", { exact: true }).first()).toBeVisible({ timeout: 20_000 });

      await page.getByRole("tab", { name: "Logs" }).click();
      await page.getByRole("combobox", { name: "Origin" }).click();
      await page.getByRole("option", { name: "Schedule", exact: true }).click();
      await expect(page.getByRole("cell", { name: "Schedule", exact: true }).first()).toBeVisible({
        timeout: 20_000,
      });
    });

    await test.step("disable from the Schedules UI", async () => {
      await openSchedules(page);
      const row = page.getByTestId(`schedules-row-${scheduleId}`);
      await expect(row).toBeVisible({ timeout: 15_000 });
      await page.getByRole("button", { name: `Disable ${scheduleId}` }).click();
      await expect(page.getByRole("button", { name: `Enable ${scheduleId}` })).toBeVisible({
        timeout: 20_000,
      });
      await expect(row.getByText("Disabled", { exact: true })).toBeVisible();

      await page.locator("#schedules-filter").selectOption("enabled");
      await expect(row).toHaveCount(0);
      await page.locator("#schedules-filter").selectOption("disabled");
      await expect(page.getByTestId(`schedules-row-${scheduleId}`)).toBeVisible();
    });

    await test.step("SQL Studio shows hits and procedure_logs from the schedule", async () => {
      await openSqlStudio(page);
      await openFreshSqlTab(page);
      await runSql(page, `SELECT COUNT(*) AS n FROM ${ns}.schedule_hits;`);
      const counts = await readSqlRows(page, ["n"], 1);
      expect(Number(counts[0]?.n)).toBeGreaterThanOrEqual(1);

      await runSql(
        page,
        `SELECT origin, outcome, schedule_id FROM system.procedure_logs WHERE schedule_id = '${scheduleId}' AND origin = 'schedule' AND outcome = 'ok';`,
      );
      const logs = await readSqlRows(page, ["origin", "outcome", "schedule_id"]);
      expect(logs.some((row) => row.origin === "schedule" && row.outcome === "ok" && row.schedule_id === scheduleId)).toBe(true);
    });
  } finally {
    await openSqlStudio(page).catch(() => undefined);
    await openFreshSqlTab(page).catch(() => undefined);
    await runSql(page, `DROP NAMESPACE IF EXISTS ${ns} CASCADE;`).catch(() => undefined);
  }
});

test("Create schedule opens SQL Studio with a CREATE SCHEDULE template", async ({ page }) => {
  await skipUnlessLiveAdminUi(page);
  await openSchedules(page);
  await page.getByRole("button", { name: "Create schedule" }).click();
  await expect(page.getByRole("button", { name: /^Run(?: selected)?$/ })).toBeVisible({
    timeout: 60_000,
  });
  await expect(page.getByText("CREATE SCHEDULE").first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText("EXECUTE PROCEDURE").first()).toBeVisible();
});
