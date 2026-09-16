import { expect, type Page, test } from "@playwright/test";

const env = (globalThis as { process?: { env?: Record<string, string | undefined> } })
  .process?.env;

export const backendOrigin = (
  env?.KALAMDB_E2E_BACKEND_URL
  ?? env?.VITE_API_URL
  ?? env?.KALAMDB_SERVER_URL
  ?? env?.KALAMDB_URL
  ?? env?.KALAM_URL
  ?? "http://127.0.0.1:2900"
).replace("://localhost", "://127.0.0.1");

const backendRequestTimeoutMs = 8_000;
export const sqlTimeoutMs = 45_000;

export function uniqueName(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}${Math.floor(Math.random() * 1_000).toString(36)}`;
}

export async function backendAvailable(page: Page): Promise<boolean> {
  try {
    const response = await page.request.get(`${backendOrigin}/v1/api/auth/status`, {
      timeout: backendRequestTimeoutMs,
    });
    return response.ok();
  } catch {
    return false;
  }
}

async function isSignedIn(page: Page): Promise<boolean> {
  return page
    .locator('a[href="/ui/sql"]')
    .waitFor({ state: "visible", timeout: 12_000 })
    .then(() => true)
    .catch(() => false);
}

export async function loginAdminUi(page: Page): Promise<boolean> {
  const candidates = [
    {
      user: env?.KALAMDB_E2E_ADMIN_USER ?? "root",
      password: env?.KALAMDB_E2E_ADMIN_PASSWORD ?? env?.KALAMDB_ROOT_PASSWORD ?? "kalamdb123",
    },
    {
      user: "root",
      password: env?.KALAMDB_ROOT_PASSWORD ?? "mypass",
    },
    {
      user: "admin",
      password: env?.KALAMDB_E2E_ADMIN_PASSWORD ?? env?.KALAMDB_ROOT_PASSWORD ?? "kalamdb123",
    },
  ];

  await page.goto("/ui/login");
  if (await isSignedIn(page)) {
    return true;
  }

  for (const credentials of candidates) {
    const username = page.locator("#username");
    const usernameReady = await username
      .waitFor({ state: "visible", timeout: 10_000 })
      .then(() => true)
      .catch(() => false);
    if (!usernameReady) {
      return isSignedIn(page);
    }

    await username.fill(credentials.user);
    await page.locator("#password").fill(credentials.password);
    await page.getByRole("button", { name: "Log in" }).click();
    if (await isSignedIn(page)) {
      return true;
    }
  }

  return false;
}

export async function skipUnlessLiveAdminUi(page: Page): Promise<void> {
  test.skip(!(await backendAvailable(page)), `KalamDB backend is not reachable at ${backendOrigin}`);
  test.skip(!(await loginAdminUi(page)), "admin credentials could not log in to the Admin UI");
}

export async function clickNav(page: Page, href: string): Promise<void> {
  await page.locator(`a[href="${href}"]`).first().click();
}

export async function openSqlStudio(page: Page): Promise<void> {
  await clickNav(page, "/ui/sql");
  await expect(page.getByText("Loading SQL workspace...")).toHaveCount(0, { timeout: 30_000 });
  await expect(page.getByRole("button", { name: /^Run(?: selected)?$/ })).toBeVisible({
    timeout: 30_000,
  });
}

export async function openFreshSqlTab(page: Page): Promise<void> {
  await openSqlStudio(page);
  const newTab = page.getByRole("button", { name: "New query tab" });
  if (!(await newTab.isVisible().catch(() => false))) {
    return;
  }
  await newTab.click();
  await expect(page.getByRole("button", { name: /^Run(?: selected)?$/ })).toBeVisible({
    timeout: 10_000,
  });
}

export async function fillMonaco(page: Page, sql: string): Promise<void> {
  await expect(page.locator(".monaco-editor")).toBeVisible({ timeout: 30_000 });
  await page.waitForFunction(() => {
    const monaco = (window as unknown as { monaco?: { editor?: { getEditors?: () => unknown[] } } }).monaco;
    return Boolean(monaco?.editor?.getEditors?.()?.length);
  }, undefined, { timeout: 30_000 });

  await page.locator(".monaco-editor textarea").last().click({ force: true });

  await page.evaluate((nextSql) => {
    const monaco = (window as unknown as {
      monaco: {
        editor: {
          getEditors: () => Array<{
            setValue: (value: string) => void;
            getValue: () => string;
            layout?: () => void;
            focus?: () => void;
            getDomNode?: () => HTMLElement | null;
          }>;
        };
      };
    }).monaco;
    const editors = monaco.editor.getEditors();
    const editor = editors.find((item) => {
      const node = item.getDomNode?.();
      if (!node?.isConnected) {
        return false;
      }
      const rect = node.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    }) ?? editors[editors.length - 1];
    editor.setValue(nextSql);
    editor.focus?.();
    editor.layout?.();
  }, sql);

  const marker = sql
    .trim()
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length >= 12) ?? sql.trim();
  await expect(page.locator(".view-line").filter({ hasText: marker.slice(0, 72) }).first()).toBeVisible({
    timeout: 10_000,
  });
}

export async function runSql(page: Page, sql: string): Promise<void> {
  await fillMonaco(page, sql);
  const runButton = page.getByRole("button", { name: /^Run(?: selected)?$/ });
  await expect(runButton).toBeEnabled({ timeout: 10_000 });
  await runButton.click();
  await page.getByText("Running query...").waitFor({ state: "visible", timeout: 5_000 }).catch(() => undefined);
  await expect(page.getByText("Running query...")).toHaveCount(0, { timeout: sqlTimeoutMs });
  const resultsTab = page.getByRole("tab", { name: /^Results/ });
  if (await resultsTab.isVisible().catch(() => false)) {
    await resultsTab.click();
  }
  const failure = page.getByText("Execution failed", { exact: true });
  if (await failure.isVisible().catch(() => false)) {
    const detail = await failure.locator("xpath=..").innerText().catch(() => "");
    throw new Error(`SQL execution failed:\n${detail}`);
  }
}

export function sqlCell(page: Page, rowIndex: number, columnName: string) {
  return page.locator(`[data-row-index="${rowIndex}"][data-column-name="${columnName}"]`);
}

export async function readSqlRows(
  page: Page,
  columns: string[],
  expectedCount?: number,
): Promise<Record<string, string>[]> {
  await expect(sqlCell(page, 0, columns[0])).toBeVisible({ timeout: 15_000 });
  const rows: Record<string, string>[] = [];
  for (let rowIndex = 0; rowIndex < (expectedCount ?? 20); rowIndex += 1) {
    const firstCell = sqlCell(page, rowIndex, columns[0]);
    if (!(await firstCell.isVisible().catch(() => false))) {
      break;
    }
    const row: Record<string, string> = {};
    for (const column of columns) {
      row[column] = (await sqlCell(page, rowIndex, column).innerText()).trim();
    }
    rows.push(row);
  }
  if (expectedCount !== undefined) {
    expect(rows).toHaveLength(expectedCount);
  }
  return rows;
}
