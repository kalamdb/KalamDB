import { expect, type Page, test } from "@playwright/test";

type AuthUser = {
  id: string;
  username: string;
  role: string;
  email: string;
  created_at: string;
  updated_at: string;
};

type SchemaRow = Record<string, unknown>;

const namespaceSchemaKeys = [
  "namespace_id",
  "name",
  "created_at",
  "options",
  "table_count",
];

const tableSchemaKeys = [
  "table_id",
  "table_name",
  "namespace_id",
  "table_type",
  "created_at",
  "schema_version",
  "columns",
  "table_comment",
  "updated_at",
  "options",
  "access_level",
  "is_latest",
  "storage_id",
  "use_user_storage",
];

const adminUser: AuthUser = {
  id: "admin-user",
  username: "admin@example.org",
  role: "dba",
  email: "admin@example.org",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};

function loginResponse(user: AuthUser) {
  return {
    user,
    admin_ui_access: true,
    access_token: "test-token-dba",
    refresh_token: "test-refresh-dba",
    expires_at: "2099-01-01T00:00:00Z",
    refresh_expires_at: "2099-01-02T00:00:00Z",
  };
}

function schemaResponse(rows: SchemaRow[], keys?: string[]) {
  const responseKeys = keys ?? Object.keys(rows[0] ?? { ok: true });
  return {
    status: "success",
    took: 1,
    results: [
      {
        schema: responseKeys.map((name, index) => ({
          name,
          index,
          data_type: "Utf8",
        })),
        named_rows: rows,
        row_count: rows.length,
      },
    ],
  };
}

function successResponse(message = "ok") {
  return {
    status: "success",
    took: 1,
    results: [{ schema: [], named_rows: [], rows: [], row_count: 0, message }],
  };
}

function defaultSchemaRows(): SchemaRow[] {
  return [
    {
      table_id: "default.settings",
      namespace_id: "default",
      table_name: "settings",
      table_type: "SHARED",
      storage_id: "local",
      access_level: "PRIVATE",
      use_user_storage: false,
      schema_version: 1,
      is_latest: true,
      options: {
        table_type: "SHARED",
        storage_id: "local",
        access_level: "private",
        flush_policy: { type: "row_limit", row_limit: 1000 },
        compression: "snappy",
      },
      columns: [
        {
          column_name: "id",
          data_type: "BIGINT",
          is_nullable: false,
          is_primary_key: true,
          ordinal_position: 1,
        },
        {
          column_name: "value",
          data_type: "TEXT",
          is_nullable: false,
          is_primary_key: false,
          ordinal_position: 2,
        },
      ],
      table_comment: null,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    },
  ];
}

function extractSqlFromRequestBody(body: string): string {
  try {
    const parsed = JSON.parse(body) as { sql?: unknown; query?: unknown };
    const sql = typeof parsed.sql === "string" ? parsed.sql : parsed.query;
    if (typeof sql === "string") return sql;
  } catch {
    // Fall back to multipart/plain body parsing below.
  }

  const multipartSql = body.match(/name="sql"\r?\n\r?\n([\s\S]*?)\r?\n--/);
  if (multipartSql?.[1]) return multipartSql[1].trim();
  return body;
}

async function mockAdminApi(page: Page, executedSql: string[]) {
  let schemaRows = defaultSchemaRows();

  await page.route("**/v1/api/**", async (route) => {
    const requestUrl = new URL(route.request().url());
    const path = requestUrl.pathname.replace(/^\/v1\/api/, "");

    if (path === "/auth/status") {
      await route.fulfill({ json: { needs_setup: false } });
      return;
    }

    if (path === "/auth/login-options") {
      await route.fulfill({
        json: { local: { enabled: true }, oidc: { enabled: false } },
      });
      return;
    }

    if (path === "/auth/refresh") {
      await route.fulfill({ json: loginResponse(adminUser) });
      return;
    }

    if (path === "/auth/me") {
      await route.fulfill({ json: { user: adminUser, admin_ui_access: true } });
      return;
    }

    if (path === "/sql") {
      const body = route.request().postData() ?? "";
      const sql = extractSqlFromRequestBody(body);
      const normalized = sql.toLowerCase().replace(/["`]/g, "");

      if (normalized.includes("system.namespaces")) {
        await route.fulfill({
          json: schemaResponse(
            [
              {
                namespace_id: "default",
                name: "default",
                created_at: "2026-01-01T00:00:00Z",
                options: null,
                table_count: schemaRows.length,
              },
            ],
            namespaceSchemaKeys,
          ),
        });
        return;
      }

      if (normalized.includes("system.schemas")) {
        await route.fulfill({
          json: schemaResponse(schemaRows, tableSchemaKeys),
        });
        return;
      }

      if (normalized.includes("dba.favorites")) {
        await route.fulfill({ json: schemaResponse([]) });
        return;
      }

      if (
        normalized.includes("count(*)") ||
        normalized.includes('count("*")')
      ) {
        await route.fulfill({ json: schemaResponse([{ c: 0 }]) });
        return;
      }

      if (
        normalized.includes("create table") ||
        normalized.includes("alter table")
      ) {
        executedSql.push(sql);
        if (normalized.includes("create table")) {
          schemaRows = [
            ...schemaRows,
            {
              table_id: "default.audit_stream",
              namespace_id: "default",
              table_name: "audit_stream",
              table_type: "STREAM",
              schema_version: 1,
              is_latest: true,
              storage_id: null,
              access_level: null,
              use_user_storage: false,
              options: {
                table_type: "STREAM",
                ttl_seconds: 7200,
                eviction_strategy: "hybrid",
                max_stream_size_bytes: 1048576,
                compression: "lz4",
              },
              columns: [
                {
                  column_name: "id",
                  data_type: "BIGINT",
                  is_nullable: false,
                  is_primary_key: true,
                  ordinal_position: 1,
                },
              ],
              table_comment: null,
              created_at: "2026-01-01T00:00:00Z",
              updated_at: "2026-01-01T00:00:00Z",
            },
          ];
        }
        await route.fulfill({ json: successResponse("statement executed") });
        return;
      }

      await route.fulfill({ json: successResponse() });
      return;
    }

    await route.fulfill({ json: {} });
  });
}

async function openTableEditor(page: Page) {
  await page.goto("/ui/sql");
  await expect(page.getByRole("button", { name: /editor/i })).toBeVisible();
  await page.getByRole("button", { name: /editor/i }).click();
}

async function chooseSelect(page: Page, testId: string, optionName: RegExp) {
  await page.getByTestId(testId).click();
  await page.getByRole("option", { name: optionName }).click();
}

test("admin creates a stream table with stream-specific options", async ({
  page,
}) => {
  const executedSql: string[] = [];
  await mockAdminApi(page, executedSql);
  await openTableEditor(page);

  await page.getByRole("button", { name: /new table/i }).click();
  await chooseSelect(page, "table-type-select", /^stream$/i);
  await page.getByPlaceholder("e.g. users").fill("audit_stream");
  await page.getByTestId("table-option-ttl-seconds").fill("7200");
  await chooseSelect(page, "table-option-eviction-strategy", /^hybrid$/i);
  await page.getByTestId("table-option-max-stream-size").fill("1048576");
  await chooseSelect(page, "table-option-compression", /^lz4$/i);

  await page.getByRole("button", { name: /review & create/i }).click();
  await page.getByRole("button", { name: /^commit$/i }).click();

  await expect
    .poll(() => executedSql.find((sql) => /create\s+table/i.test(sql)) ?? "")
    .toContain("TYPE = 'STREAM'");
  const createSql =
    executedSql.find((sql) => /create\s+table/i.test(sql)) ?? "";
  expect(createSql).toContain("TTL_SECONDS = 7200");
  expect(createSql).toContain("EVICTION_STRATEGY = 'hybrid'");
  expect(createSql).toContain("MAX_STREAM_SIZE_BYTES = 1048576");
  expect(createSql).toContain("COMPRESSION = 'lz4'");
});

test("admin edits shared table options", async ({ page }) => {
  const executedSql: string[] = [];
  await mockAdminApi(page, executedSql);
  await openTableEditor(page);

  await page.getByRole("button", { name: /^settings$/i }).click();
  await chooseSelect(page, "table-option-access-level", /^public$/i);
  await chooseSelect(page, "table-option-flush-policy", /^combined$/i);
  await page.getByTestId("table-option-flush-rows").fill("2000");
  await page.getByTestId("table-option-flush-interval").fill("120");
  await chooseSelect(page, "table-option-compression", /^zstd$/i);

  await page.getByRole("button", { name: /review & save/i }).click();
  await page.getByRole("button", { name: /^commit$/i }).click();

  await expect
    .poll(() => executedSql.find((sql) => /alter\s+table/i.test(sql)) ?? "")
    .toContain("SET TBLPROPERTIES");
  const alterSql = executedSql.find((sql) => /alter\s+table/i.test(sql)) ?? "";
  expect(alterSql).toContain("ACCESS_LEVEL = 'PUBLIC'");
  expect(alterSql).toContain("FLUSH_POLICY = 'rows:2000,interval:120'");
  expect(alterSql).toContain("COMPRESSION = 'zstd'");
});
