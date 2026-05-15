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
}

async function fetchToken(): Promise<TokenResponse> {
  const res = await fetch("/api/auth/token", { method: "POST" });
  if (!res.ok) {
    throw new Error(`Failed to fetch KalamDB token (${res.status})`);
  }
  return (await res.json()) as TokenResponse;
}

let singleton: KalamDBClient | null = null;

export function getClient(): KalamDBClient {
  if (singleton) return singleton;
  singleton = createClient({
    url: new URL("/kdb", window.location.origin).toString(),
    authProvider: async () => {
      const { token } = await fetchToken();
      return Auth.jwt(token);
    },
    disableCompression: true,
  });
  return singleton;
}
