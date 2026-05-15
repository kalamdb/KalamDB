import "dotenv/config";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

// Tiny HTTP server that owns the KalamDB credentials. The browser never sees
// them — it calls /api/auth/token, this server logs into KalamDB with the
// bundled root credentials, and hands the resulting access token back.
//
// In a real app, /api/auth/token is where YOUR auth lives: validate the user's
// session cookie / OAuth token first, then mint a per-user KalamDB token
// scoped to that user's data. This starter demonstrates the boundary
// (secrets stay server-side) without implementing user accounts.

const URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:8080";
const USER = process.env.KALAMDB_USER ?? "root";
const PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";
const PORT = Number(process.env.PORT ?? "3001");

interface CachedToken {
  token: string;
  expiresAt: number;
}

let cached: CachedToken | null = null;
const TOKEN_REFRESH_SAFETY_MS = 60_000;

async function getKalamToken(): Promise<CachedToken> {
  if (cached && cached.expiresAt - Date.now() > TOKEN_REFRESH_SAFETY_MS) {
    return cached;
  }
  const res = await fetch(`${URL}/v1/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: USER, password: PASSWORD }),
  });
  if (!res.ok) {
    throw new Error(`KalamDB login failed (${res.status}): ${await res.text().catch(() => "")}`);
  }
  const body = (await res.json()) as { access_token: string; expires_in?: number };
  const lifetimeMs = (body.expires_in ?? 60 * 60) * 1000;
  cached = { token: body.access_token, expiresAt: Date.now() + lifetimeMs };
  return cached;
}

function json(res: ServerResponse, status: number, payload: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(payload));
}

async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
  if (req.method === "POST" && req.url === "/api/auth/token") {
    // Real app: authenticate the caller here (session cookie, OAuth, etc).
    // We skip that step in the starter to keep the focus on the architecture.
    const { token, expiresAt } = await getKalamToken();
    json(res, 200, { token, expiresAt });
    return;
  }
  if (req.method === "GET" && req.url === "/api/health") {
    json(res, 200, { ok: true });
    return;
  }
  json(res, 404, { error: "not_found" });
}

const server = createServer((req, res) => {
  handle(req, res).catch((err) => {
    console.error("[server] handler error:", err);
    json(res, 500, { error: err instanceof Error ? err.message : String(err) });
  });
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`[server] listening on http://127.0.0.1:${PORT}`);
});

function shutdown(signal: string): void {
  console.log(`\n[server] ${signal} — shutting down`);
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 3000).unref();
}
process.on("SIGINT", () => shutdown("SIGINT"));
process.on("SIGTERM", () => shutdown("SIGTERM"));
