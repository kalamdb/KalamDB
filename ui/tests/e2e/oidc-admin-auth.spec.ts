import { expect, type Page, test } from "@playwright/test";
import { createHash } from "node:crypto";

const env = (globalThis as { process?: { env?: Record<string, string | undefined> } })
  .process?.env;
const uiPort = Number(env?.KALAMDB_UI_PLAYWRIGHT_PORT ?? 4175);
const uiOrigin = `http://127.0.0.1:${uiPort}`;
const backendOrigin = env?.KALAMDB_E2E_BACKEND_URL ?? env?.VITE_API_URL ?? "http://localhost:2900";
const inviteTtlMs = 7 * 24 * 60 * 60 * 1000;

type AuthUser = {
  id: string;
  username: string;
  role: string;
  email: string;
  created_at: string;
  updated_at: string;
};

function oidcUser(role: "dba" | "service"): AuthUser {
  return {
    id: `e_${role}`,
    username: `${role}@example.org`,
    role,
    email: `${role}@example.org`,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function loginResponse(user: AuthUser) {
  const adminUiAccess = user.role === "dba" || user.role === "system";
  return {
    user,
    admin_ui_access: adminUiAccess,
    access_token: `test-token-${user.role}`,
    refresh_token: `test-refresh-${user.role}`,
    expires_at: "2099-01-01T00:00:00Z",
    refresh_expires_at: "2099-01-02T00:00:00Z",
  };
}

function inviteUserId(email: string): string {
  return `invite_${createHash("sha256").update(email.trim().toLowerCase()).digest("hex").slice(0, 32)}`;
}

async function backendAvailable(page: Page): Promise<boolean> {
  try {
    const response = await page.request.get(`${backendOrigin}/v1/api/auth/status`);
    return response.ok();
  } catch {
    return false;
  }
}

async function dexAvailable(page: Page): Promise<boolean> {
  try {
    const response = await page.request.get("http://127.0.0.1:5556/.well-known/openid-configuration");
    return response.ok();
  } catch {
    return false;
  }
}

async function loginAdmin(page: Page): Promise<string | null> {
  const candidates = [
    {
      user: env?.KALAMDB_E2E_ADMIN_USER ?? "root",
      password: env?.KALAMDB_E2E_ADMIN_PASSWORD ?? env?.KALAMDB_ROOT_PASSWORD ?? "kalamdb123",
    },
    {
      user: "admin",
      password: env?.KALAMDB_E2E_ADMIN_PASSWORD ?? env?.KALAMDB_ROOT_PASSWORD ?? "kalamdb123",
    },
  ];

  for (const credentials of candidates) {
    const response = await page.request.post(`${backendOrigin}/v1/api/auth/login`, {
      data: credentials,
    });
    if (!response.ok()) {
      continue;
    }

    const payload = await response.json();
    if (typeof payload.access_token === "string" && payload.access_token.length > 0) {
      return payload.access_token;
    }
  }

  return null;
}

function cellText(value: unknown): string | null {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  if (value && typeof value === "object") {
    const values = Object.values(value);
    if (values.length === 1) {
      return cellText(values[0]);
    }
  }
  return null;
}

async function executeAdminSql(page: Page, token: string, sql: string, ignoreError = false) {
  const response = await page.request.post(`${backendOrigin}/v1/api/sql`, {
    data: { sql },
    headers: {
      Authorization: `Bearer ${token}`,
    },
  });

  if (ignoreError) {
    return;
  }

  expect(response.ok()).toBeTruthy();
  const payload = await response.json();
  expect(payload.status).toBe("success");
  return payload;
}

async function queryFirstColumn(page: Page, token: string, sql: string): Promise<string[]> {
  const payload = await executeAdminSql(page, token, sql);
  const rows = payload?.results?.[0]?.rows;
  if (!Array.isArray(rows)) {
    return [];
  }

  return rows
    .map((row) => (Array.isArray(row) ? cellText(row[0]) : null))
    .filter((value): value is string => typeof value === "string" && value.length > 0);
}

async function dropUsersByEmail(page: Page, token: string, email: string) {
  const userIds = await queryFirstColumn(
    page,
    token,
    `SELECT user_id FROM system.users WHERE email = '${email}'`,
  );

  for (const userId of userIds) {
    await executeAdminSql(page, token, `DROP USER '${userId.replace(/'/g, "''")}'`, true);
  }
}

async function mockAdminApi(
  page: Page,
  user: AuthUser | null,
  exchangeUser: AuthUser | null = null,
) {
  await page.route("**/v1/api/**", async (route) => {
    const requestUrl = new URL(route.request().url());
    const path = requestUrl.pathname.replace(/^\/v1\/api/, "");

    if (path === "/auth/status") {
      await route.fulfill({ json: { needs_setup: false } });
      return;
    }

    if (path === "/auth/login-options") {
      await route.fulfill({
        json: {
          local: { enabled: true },
          oidc: {
            enabled: true,
            display_name: "Dex",
            issuer: "http://127.0.0.1:5556",
            client_id: "client",
            authorization_endpoint: "http://127.0.0.1:5556/auth",
            token_endpoint: "http://127.0.0.1:5556/token",
            scopes: ["openid", "profile", "email"],
          },
        },
      });
      return;
    }

    if (path === "/auth/refresh") {
      if (!user) {
        await route.fulfill({
          status: 401,
          json: { error: "unauthorized", message: "Not authenticated" },
        });
        return;
      }

      await route.fulfill({ json: loginResponse(user) });
      return;
    }

    if (path === "/auth/oidc/exchange-code") {
      if (!exchangeUser) {
        await route.fulfill({
          status: 401,
          json: { error: "unauthorized", message: "Invalid credentials" },
        });
        return;
      }

      await route.fulfill({ json: loginResponse(exchangeUser) });
      return;
    }

    if (path === "/auth/me") {
      const authorization = route.request().headers().authorization ?? "";
      const tokenUser =
        exchangeUser && authorization === `Bearer test-token-${exchangeUser.role}`
          ? exchangeUser
          : null;
      const currentUser = user ?? tokenUser;

      if (!currentUser) {
        await route.fulfill({
          status: 401,
          json: { error: "unauthorized", message: "Not authenticated" },
        });
        return;
      }

      await route.fulfill({
        json: {
          user: currentUser,
          admin_ui_access: currentUser.role === "dba" || currentUser.role === "system",
        },
      });
      return;
    }

    await route.fulfill({ json: {} });
  });
}

test("OIDC login button completes callback through the KalamDB backend exchange", async ({ page }) => {
  test.skip(!(await dexAvailable(page)), "local Dex is not reachable at http://127.0.0.1:5556");

  await mockAdminApi(page, null, oidcUser("dba"));

  await page.goto("/ui/login");
  await expect(page.getByRole("button", { name: /continue with dex/i })).toBeVisible();

  const authorizeRequestPromise = page.waitForRequest("http://127.0.0.1:5556/auth**");
  await page.getByRole("button", { name: /continue with dex/i }).click();
  const authorizeRequest = await authorizeRequestPromise;
  const authorizeUrl = new URL(authorizeRequest.url());
  expect(authorizeUrl.searchParams.get("client_id")).toBe("client");
  expect(authorizeUrl.searchParams.get("redirect_uri")).toBe(`${uiOrigin}/ui/oauth/callback`);
  expect(authorizeUrl.searchParams.get("code_challenge_method")).toBe("S256");

  const connectorButton = page.getByRole("button", { name: /log in with email/i });
  if (await connectorButton.isVisible().catch(() => false)) {
    await connectorButton.click();
  }

  await page.locator('input[name="login"]').fill("alice@example.org");
  await page.locator('input[name="password"]').fill("kalamdb123");
  await page.locator('button[type="submit"]').click();

  await expect(page.getByRole("link", { name: /sql studio/i })).toBeVisible();
});

test("invited Dex user is created from the OIDC email invite on first Admin UI login", async ({ page }) => {
  test.skip(!(await backendAvailable(page)), `KalamDB backend is not reachable at ${backendOrigin}`);
  test.skip(!(await dexAvailable(page)), "local Dex is not reachable at http://127.0.0.1:5556");

  const adminToken = await loginAdmin(page);
  test.skip(!adminToken, "admin credentials could not log in to KalamDB");
  if (!adminToken) {
    return;
  }

  const inviteEmail = "heidi@example.org";
  const inviteId = inviteUserId(inviteEmail);
  const expiresAt = Date.now() + inviteTtlMs;

  await dropUsersByEmail(page, adminToken, inviteEmail);
  await executeAdminSql(page, adminToken, `DROP USER '${inviteId}'`, true);
  await executeAdminSql(
    page,
    adminToken,
    `CREATE USER INVITE '${inviteEmail}' ROLE 'dba' EXPIRES_AT ${expiresAt}`,
  );

  await page.goto("/ui/login");
  await page.getByRole("button", { name: /continue with dex/i }).click();

  const connectorButton = page.getByRole("button", { name: /log in with email/i });
  if (await connectorButton.isVisible().catch(() => false)) {
    await connectorButton.click();
  }

  await page.locator('input[name="login"]').fill(inviteEmail);
  await page.locator('input[name="password"]').fill("kalamdb123");
  await page.locator('button[type="submit"]').click();

  await expect(page.getByRole("link", { name: /sql studio/i })).toBeVisible();
  await page.getByRole("link", { name: /^users$/i }).click();
  const users = page.getByRole("region", { name: /users list/i });
  const invites = page.getByRole("region", { name: /pending invites/i });

  await expect(users.getByText(inviteEmail)).toBeVisible();
  await expect(users.getByRole("row", { name: new RegExp(`${inviteEmail}.*dba|dba.*${inviteEmail}`) })).toBeVisible();
  await expect(invites.getByText(inviteId)).toHaveCount(0);

  await dropUsersByEmail(page, adminToken, inviteEmail);
});

test("OIDC dba users can enter the Admin UI", async ({ page }) => {
  await mockAdminApi(page, oidcUser("dba"));

  await page.goto("/ui/dashboard");

  await expect(page.getByRole("link", { name: /sql studio/i })).toBeVisible();
  await expect(page.getByText("Access Denied")).toHaveCount(0);
});

test("OIDC service users authenticate but cannot enter the Admin UI", async ({ page }) => {
  await mockAdminApi(page, oidcUser("service"));

  await page.goto("/ui/dashboard");

  await expect(page.getByRole("heading", { name: "Access Denied" })).toBeVisible();
  await expect(page.getByText(/current role:/i)).toContainText("service");
});
