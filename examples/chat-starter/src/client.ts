import { Auth, createClient, type KalamDBClient } from "@kalamdb/client";

// The browser holds no KalamDB credentials. It fetches a short-lived token
// from /api/auth/token (served by the backend in server/index.ts) and uses
// that to authenticate to KalamDB directly for live queries and SQL.
//
// authProvider is called by the SDK whenever it needs to (re)authenticate —
// initially, and again if a request fails with 401. Refreshing the token is
// therefore a re-fetch from /api/auth/token; no client-side timer needed.

interface TokenResponse {
  token: string;
  expiresAt: number;
  user: string;
}

async function fetchToken(user: string): Promise<TokenResponse> {
  const res = await fetch("/api/auth/token", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ user }),
  });
  if (!res.ok) {
    throw new Error(`Failed to fetch KalamDB token (${res.status})`);
  }
  return (await res.json()) as TokenResponse;
}

/**
 * Build a fresh KalamDB client scoped to `user`. A new client is created on
 * every call so switching users in the UI yields fresh live-query
 * subscriptions under the new identity — never re-using the previous user's
 * auth state.
 */
export function createKalamClient(user: string): KalamDBClient {
  return createClient({
    url: new URL("/kdb", window.location.origin).toString(),
    authProvider: async () => {
      const { token } = await fetchToken(user);
      return Auth.jwt(token);
    },
    disableCompression: true,
  });
}
