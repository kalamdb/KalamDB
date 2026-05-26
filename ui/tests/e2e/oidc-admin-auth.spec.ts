import { expect, type Page, test } from "@playwright/test";

const env = (globalThis as { process?: { env?: Record<string, string | undefined> } })
  .process?.env;
const uiPort = Number(env?.KALAMDB_UI_PLAYWRIGHT_PORT ?? 4175);
const uiOrigin = `http://127.0.0.1:${uiPort}`;

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
  let dexAvailable = false;
  try {
    const dexStatus = await page.request.get(
      "http://127.0.0.1:5556/.well-known/openid-configuration",
    );
    dexAvailable = dexStatus.ok();
  } catch {
    dexAvailable = false;
  }
  test.skip(!dexAvailable, "local Dex is not reachable at http://127.0.0.1:5556");

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