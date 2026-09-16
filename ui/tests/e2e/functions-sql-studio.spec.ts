import { expect, type Page, test } from "@playwright/test";
import { clickNav, openSqlStudio, readSqlRows, runSql, skipUnlessLiveAdminUi, uniqueName } from "./helpers";

test.describe.configure({ timeout: 180_000 });

async function refreshProcedureCatalog(page: Page): Promise<void> {
  const refresh = page.getByRole("button", { name: "Refresh" });
  await expect(refresh).toBeVisible({ timeout: 15_000 });
  await refresh.click();
  await expect(refresh).toBeEnabled({ timeout: 30_000 });
}

async function openFunctionTest(page: Page, procedureId: string): Promise<void> {
  await clickNav(page, "/ui/functions");
  await expect(page.getByPlaceholder("Search functions...")).toBeVisible({ timeout: 30_000 });
  await refreshProcedureCatalog(page);
  await page.getByPlaceholder("Search functions...").fill(procedureId);
  const row = page.getByRole("row").filter({ hasText: procedureId });
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.click();
  await page.getByRole("tab", { name: "Test" }).click();
  await expect(page.getByRole("button", { name: "Run test" })).toBeVisible({ timeout: 15_000 });

  const enumSelect = page.getByRole("combobox", { name: "status" });
  if (!(await enumSelect.isVisible().catch(() => false))) {
    await refreshProcedureCatalog(page);
    await page.getByRole("tab", { name: "Test" }).click();
  }
  await expect(enumSelect).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText(/No enum labels found/)).toHaveCount(0);
}

async function fillLabeledInput(page: Page, name: string, value: string): Promise<void> {
  const field = page.getByRole("textbox", { name: new RegExp(name) }).or(
    page.getByRole("spinbutton", { name: new RegExp(name) }),
  );
  await expect(field).toBeVisible({ timeout: 15_000 });
  await field.fill(value);
}

function uniqueNamespace(): string {
  return uniqueName("concerts_e2e");
}

function deploySql(ns: string): string {
  return `
CREATE NAMESPACE ${ns};

CREATE TABLE ${ns}.venues (
  venue_id TEXT PRIMARY KEY,
  city TEXT NOT NULL,
  capacity INT NOT NULL
) WITH (TYPE = 'SHARED');

INSERT INTO ${ns}.venues (venue_id, city, capacity) VALUES ('hall-a', 'Cairo', 1200);

CREATE TYPE ${ns}.ticket_status AS ENUM ('hold', 'confirmed', 'cancelled');
CREATE TYPE ${ns}.venue FROM TABLE ${ns}.venues;
CREATE TYPE ${ns}.guest AS (
  display_name TEXT NOT NULL,
  vip BOOLEAN NOT NULL
);
CREATE TYPE ${ns}.booking AS (
  confirmation_id TEXT NOT NULL,
  venue ${ns}.venue NOT NULL,
  guest ${ns}.guest NOT NULL,
  status ${ns}.ticket_status NOT NULL,
  seats INT NOT NULL,
  notes TEXT
);

CREATE OR REPLACE PROCEDURE ${ns}.book_tickets(
  venue_id TEXT NOT NULL,
  display_name TEXT NOT NULL,
  vip BOOLEAN NOT NULL,
  status ${ns}.ticket_status NOT NULL,
  seats INT NOT NULL,
  notes TEXT
)
RETURNS ${ns}.booking
LANGUAGE JAVASCRIPT
COMMENT 'Playwright concert booking'
AS $$
  var venueId = input.venue_id || input.venueId;
  var guestName = input.display_name || input.displayName;
  var vip = input.vip;
  var status = input.status;
  var seats = input.seats;
  var notes = input.notes;
  return ctx.db.query(
    "SELECT venue_id, city, capacity, _seq, _deleted FROM ${ns}.venues WHERE venue_id = $1",
    [venueId]
  ).then(function (rows) {
    var row = rows && rows[0];
    if (!row) {
      throw new Error("unknown venue");
    }
    var resolvedVenueId = row.venue_id || row.venueId;
    var city = row.city;
    var capacity = row.capacity;
    ctx.log.info("booking", { venue: resolvedVenueId, status: status });
    return {
      confirmation_id: "BK-" + String(resolvedVenueId).toUpperCase() + "-42",
      venue: {
        venue_id: resolvedVenueId,
        city: city,
        capacity: capacity,
        _seq: row._seq,
        _deleted: row._deleted === true
      },
      guest: {
        display_name: guestName,
        vip: vip
      },
      status: status,
      seats: seats,
      notes: notes
    };
  });
$$;

CREATE PROCEDURE ${ns}.later() RETURNS TEXT COMMENT 'Not implemented yet';
`.trim();
}

function callSql(ns: string): string {
  return `CALL ${ns}.book_tickets('hall-a', 'Ada Lovelace', TRUE, 'confirmed', 2, 'aisle');`;
}

function catalogSql(ns: string): string {
  return `
SELECT procedure_id, schema, name, signature, return_type, implementation, module_id,
       revision_id, security, owner, grants, comment, language, source
FROM system.procedures
WHERE schema = '${ns}'
ORDER BY name
`.trim();
}

async function openFunctionsList(page: Page, search: string): Promise<void> {
  await clickNav(page, "/ui/functions");
  await expect(page.getByPlaceholder("Search functions...")).toBeVisible({ timeout: 30_000 });
  await refreshProcedureCatalog(page);
  await page.getByPlaceholder("Search functions...").fill(search);
}

test("SQL Studio deploys a typed booking function and Test UI can invoke it", async ({ page }) => {
  await skipUnlessLiveAdminUi(page);

  const ns = uniqueNamespace();
  const procedureId = `${ns}.book_tickets`;

  try {
    await openSqlStudio(page);

    await test.step("create enum, table row type, request object, and deploy procedure", async () => {
      await runSql(page, deploySql(ns));
      await page.getByRole("tab", { name: /^Log/ }).click();
      await expect(page.getByText(/created|replaced|INSERT/i).first()).toBeVisible({ timeout: 10_000 });
    });

    await test.step("system.procedures lists implementation, signature, and inline source", async () => {
      await openSqlStudio(page);
      await runSql(page, catalogSql(ns));
      const rows = await readSqlRows(
        page,
        [
          "procedure_id",
          "name",
          "signature",
          "return_type",
          "implementation",
          "revision_id",
          "security",
          "comment",
          "language",
          "source",
        ],
        2,
      );
      const later = rows.find((row) => row.name === "later");
      const booking = rows.find((row) => row.name === "book_tickets");
      expect(later?.implementation).toBe("missing");
      expect(later?.comment).toContain("Not implemented yet");
      expect(["", "null", "NULL", "\u2014"]).toContain((later?.source ?? "").trim());
      expect(booking?.implementation).toBe("inline");
      expect(booking?.signature).toContain("venue_id");
      expect(booking?.return_type).toContain("booking");
      expect(booking?.comment).toContain("Playwright concert booking");
      expect(booking?.language.toLowerCase()).toContain("javascript");
      expect(booking?.revision_id).toMatch(/^inline:/);
      expect(booking?.source).toContain("venue_id");
      expect(booking?.security.toLowerCase()).toContain("invoker");
    });

    await test.step("CALL the procedure from SQL Studio", async () => {
      await runSql(page, callSql(ns));
      await page.getByRole("tab", { name: /^Results/ }).click();
      await expect(page.getByTestId("results-column-header-result")).toBeVisible({ timeout: 15_000 });
      await page.locator('[data-column-name="result"]').first().dblclick();
      await expect(page.getByRole("dialog")).toContainText(/BK-HALL-A-42|Cairo|confirmed|Ada Lovelace/i, {
        timeout: 10_000,
      });
      await page.getByRole("button", { name: "Cancel" }).click();
    });

    await test.step("Functions list shows status and column values", async () => {
      await openFunctionsList(page, procedureId);
      await expect(page.getByRole("columnheader", { name: "Function" })).toBeVisible();
      await expect(page.getByRole("columnheader", { name: "Status" })).toBeVisible();
      await expect(page.getByRole("columnheader", { name: "Current revision" })).toBeVisible();
      await expect(page.getByRole("columnheader", { name: "Last updated" })).toBeVisible();
      await expect(page.getByRole("columnheader", { name: "Calls 24h" })).toBeVisible();

      const bookingRow = page.getByTestId(`functions-row-${procedureId}`);
      await expect(bookingRow).toBeVisible({ timeout: 30_000 });
      await expect(bookingRow).toContainText("Ready");
      await expect(bookingRow).toContainText("Playwright concert booking");
      await expect(bookingRow).toContainText("inline:");
      await expect(bookingRow.locator("td").nth(4)).toHaveText(/^\d/);

      await page.getByPlaceholder("Search functions...").fill(`${ns}.later`);
      const laterRow = page.getByTestId(`functions-row-${ns}.later`);
      await expect(laterRow).toBeVisible({ timeout: 15_000 });
      await expect(laterRow).toContainText("Unimplemented");
      await expect(laterRow).toContainText("Not implemented yet");
    });

    await test.step("function overview shows inline source", async () => {
      await openFunctionsList(page, procedureId);
      await page.getByTestId(`functions-row-${procedureId}`).click();
      await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute("data-state", "active");
      await expect(page.getByText("Inline script")).toBeVisible();
      const inlineSource = page.getByTestId("function-inline-source");
      await expect(inlineSource).toBeVisible();
      await expect(inlineSource.locator(".monaco-editor")).toBeVisible({ timeout: 30_000 });
      await expect(inlineSource.locator(".view-line")).toContainText("venue_id");
      await expect(page.getByTestId("function-runtime-memory")).toBeVisible();
    });

    await test.step("invoke the same procedure from the Functions Test UI", async () => {
      await openFunctionTest(page, procedureId);

      await fillLabeledInput(page, "venue_id", "hall-a");
      await fillLabeledInput(page, "display_name", "Ada Lovelace");
      await fillLabeledInput(page, "seats", "2");
      await fillLabeledInput(page, "notes", "aisle");

      const vip = page.getByRole("switch", { name: "vip" });
      await expect(vip).toBeVisible();
      if ((await vip.getAttribute("data-state")) !== "checked") {
        await vip.click();
      }

      await page.getByRole("combobox", { name: "status" }).click();
      await page.getByRole("option", { name: "confirmed" }).click();

      await page.getByRole("button", { name: "Run test" }).click();
      await expect(page.getByText("Ada Lovelace")).toBeVisible({ timeout: 30_000 });
      await expect(page.getByText(/BK-HALL-A-42/)).toBeVisible();
      await expect(page.getByText("Cairo")).toBeVisible();
      await expect(page.getByText('"confirmed"', { exact: true })).toBeVisible();
    });

    await test.step("overview shows isolate memory after a successful invoke", async () => {
      await page.getByRole("tab", { name: "Overview" }).click();
      const memory = page.getByTestId("function-runtime-memory");
      await expect(memory).toBeVisible();
      await expect(memory.getByText("Used heap", { exact: true })).toBeVisible({
        timeout: 15_000,
      });
      await expect(memory.getByText("Peak heap", { exact: true })).toBeVisible();
      await expect(memory.getByText("Reserved", { exact: true })).toBeVisible();
    });
  } finally {
    await openSqlStudio(page).catch(() => undefined);
    await runSql(page, `DROP NAMESPACE IF EXISTS ${ns} CASCADE;`).catch(() => undefined);
  }
});
