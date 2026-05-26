// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { buildOAuthAuthorizationUrl, consumeOAuthRedirect } from "@/lib/oauth";
import type { OidcLoginOptions } from "@/lib/api";

const provider: OidcLoginOptions = {
  enabled: true,
  display_name: "Dex",
  issuer: "https://idp.example.com/dex",
  client_id: "kalamdb-admin",
  authorization_endpoint: "https://idp.example.com/dex/auth",
  token_endpoint: "https://idp.example.com/dex/token",
  scopes: ["openid", "email", "profile"],
};

describe("OAuth PKCE helpers", () => {
  beforeEach(() => {
    sessionStorage.clear();
    vi.stubGlobal("fetch", vi.fn());
    Object.defineProperty(window, "location", {
      configurable: true,
      value: new URL("https://admin.example.com/ui/login"),
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    sessionStorage.clear();
  });

  it("builds an authorization-code PKCE URL and stores callback state", async () => {
    const url = new URL(await buildOAuthAuthorizationUrl(provider, "/sql"));

    expect(url.origin + url.pathname).toBe("https://idp.example.com/dex/auth");
    expect(url.searchParams.get("response_type")).toBe("code");
    expect(url.searchParams.get("client_id")).toBe("kalamdb-admin");
    expect(url.searchParams.get("redirect_uri")).toBe("https://admin.example.com/ui/oauth/callback");
    expect(url.searchParams.get("scope")).toBe("openid email profile");
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(url.searchParams.get("code_challenge")).toMatch(/^[A-Za-z0-9_-]+$/);

    const stored = JSON.parse(sessionStorage.getItem("kalamdb.admin.oauth.state") ?? "{}");
    expect(stored.state).toBe(url.searchParams.get("state"));
    expect(stored.codeVerifier).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(stored.returnTo).toBe("/sql");
    expect(stored.tokenEndpoint).toBe(provider.token_endpoint);
  });

  it("exchanges a valid callback code through the KalamDB backend", async () => {
    const authUrl = new URL(await buildOAuthAuthorizationUrl(provider, "/dashboard"));
    const state = authUrl.searchParams.get("state");
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ access_token: "kalamdb.access.token" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const result = await consumeOAuthRedirect("", `?code=auth-code-1&state=${state}`);

    expect(result).toEqual({ token: "kalamdb.access.token", returnTo: "/dashboard" });
    expect(fetchMock).toHaveBeenCalledWith(
      "http://localhost:2900/v1/api/auth/oidc/exchange-code",
      expect.objectContaining({
        method: "POST",
        headers: { "Content-Type": "application/json" },
        credentials: "include",
      }),
    );
    const body = JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body));
    expect(body.code).toBe("auth-code-1");
    expect(body.redirect_uri).toBe("https://admin.example.com/ui/oauth/callback");
    expect(body.code_verifier).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it("rejects an invalid state without calling the token endpoint", async () => {
    await buildOAuthAuthorizationUrl(provider, "/dashboard");

    await expect(consumeOAuthRedirect("", "?code=auth-code-1&state=wrong")).rejects.toThrow(
      /state did not match/i,
    );
    expect(fetch).not.toHaveBeenCalled();
  });
});