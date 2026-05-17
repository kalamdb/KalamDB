import "dotenv/config";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { randomUUID } from "node:crypto";
import { logger } from "../src/lib/logger.js";

// Tiny HTTP server that owns the KalamDB credentials. The browser never sees
// them — it calls /api/auth/token, this server logs into KalamDB with the
// bundled root credentials, and hands the resulting access token back.
//
// In a real app, /api/auth/token is where YOUR auth lives: validate the
// user's session cookie / OAuth token first, then mint a per-user KalamDB
// token scoped to that user's data. This starter demonstrates the boundary
// (secrets stay server-side) without implementing user accounts.
//
// To prevent the starter from being deployed unmodified as an open token
// vending machine, the server refuses to boot in NODE_ENV=production unless
// ALLOW_UNAUTHENTICATED_TOKENS=true is explicitly set — a fence the operator
// must deliberately step over.

const log = logger.child({ component: "server" });

const DEFAULTS = {
  url: "http://127.0.0.1:8080",
  user: "root",
  password: "kalamdb-dev-password",
  port: 3001,
  tokenRateLimitPerMinute: 10,
  healthRateLimitPerMinute: 60,
  healthCacheMs: 2_000,
} as const;

export interface ServerConfig {
  kalamdbUrl: string;
  kalamdbUser: string;
  kalamdbPassword: string;
  tokenRateLimitPerMinute: number;
  healthRateLimitPerMinute: number;
  healthCacheMs: number;
  /** When true, X-Forwarded-For is honored. Set behind a trusted reverse proxy only. */
  trustProxy: boolean;
}

function configFromEnv(): ServerConfig {
  return {
    kalamdbUrl: process.env.KALAMDB_URL ?? DEFAULTS.url,
    kalamdbUser: process.env.KALAMDB_USER ?? DEFAULTS.user,
    kalamdbPassword: process.env.KALAMDB_PASSWORD ?? DEFAULTS.password,
    tokenRateLimitPerMinute: Number(
      process.env.TOKEN_RATE_LIMIT_PER_MINUTE ?? DEFAULTS.tokenRateLimitPerMinute,
    ),
    healthRateLimitPerMinute: Number(
      process.env.HEALTH_RATE_LIMIT_PER_MINUTE ?? DEFAULTS.healthRateLimitPerMinute,
    ),
    healthCacheMs: Number(process.env.HEALTH_CACHE_MS ?? DEFAULTS.healthCacheMs),
    trustProxy: process.env.TRUST_PROXY === "1",
  };
}

interface CachedToken {
  token: string;
  expiresAt: number;
}

const TOKEN_REFRESH_SAFETY_MS = 60_000;

function makeTokenFetcher(cfg: ServerConfig): () => Promise<CachedToken> {
  let cached: CachedToken | null = null;
  return async function getKalamToken(): Promise<CachedToken> {
    if (cached && cached.expiresAt - Date.now() > TOKEN_REFRESH_SAFETY_MS) {
      return cached;
    }
    const res = await fetch(`${cfg.kalamdbUrl}/v1/api/auth/login`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ username: cfg.kalamdbUser, password: cfg.kalamdbPassword }),
    });
    if (!res.ok) {
      const body = await res.text().catch(() => "");
      throw new Error(`KalamDB login failed (${res.status}): ${body}`);
    }
    const body = (await res.json()) as { access_token: string; expires_in?: number };
    const lifetimeMs = (body.expires_in ?? 60 * 60) * 1000;
    cached = { token: body.access_token, expiresAt: Date.now() + lifetimeMs };
    return cached;
  };
}

// Per-IP token-bucket rate limiter. Memory-only — fine for a single process;
// for a multi-replica deployment swap to Redis or your auth proxy.
function makeRateLimiter(perMinute: number) {
  const buckets = new Map<string, { tokens: number; updatedAt: number }>();
  const capacity = perMinute;
  const refillPerMs = perMinute / 60_000;
  return function take(ip: string): boolean {
    const now = Date.now();
    const bucket = buckets.get(ip) ?? { tokens: capacity, updatedAt: now };
    const elapsed = now - bucket.updatedAt;
    bucket.tokens = Math.min(capacity, bucket.tokens + elapsed * refillPerMs);
    bucket.updatedAt = now;
    if (bucket.tokens < 1) {
      buckets.set(ip, bucket);
      return false;
    }
    bucket.tokens -= 1;
    buckets.set(ip, bucket);
    return true;
  };
}

function clientIp(req: IncomingMessage, trustProxy: boolean): string {
  // X-Forwarded-For is trivially spoofable — only trust it when explicitly
  // told there's a known reverse proxy in front (TRUST_PROXY=1). Otherwise
  // attackers rotate the header and bypass the per-IP limiter.
  if (trustProxy) {
    const fwd = req.headers["x-forwarded-for"];
    if (typeof fwd === "string" && fwd.length > 0) return fwd.split(",")[0]!.trim();
  }
  return req.socket.remoteAddress ?? "unknown";
}

function setSecurityHeaders(res: ServerResponse, requestId: string): void {
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader("X-Frame-Options", "DENY");
  res.setHeader("Referrer-Policy", "strict-origin-when-cross-origin");
  res.setHeader("X-Request-ID", requestId);
}

function json(res: ServerResponse, status: number, payload: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(payload));
}

export interface BuildServerOptions {
  config?: ServerConfig;
  tokenFetcher?: () => Promise<CachedToken>;
}

export function buildServer(opts: BuildServerOptions = {}): Server {
  const cfg = opts.config ?? configFromEnv();
  const getKalamToken = opts.tokenFetcher ?? makeTokenFetcher(cfg);
  const rateTakeToken = makeRateLimiter(cfg.tokenRateLimitPerMinute);
  const rateTakeHealth = makeRateLimiter(cfg.healthRateLimitPerMinute);

  // /api/health caches its upstream-healthy answer briefly so repeated probes
  // (k8s readiness, ELB, monitoring) don't amplify into upstream traffic.
  let healthCache: { ok: boolean; status: number; at: number; upstream?: number | string } | null =
    null;

  async function handle(
    req: IncomingMessage,
    res: ServerResponse,
    requestLog: typeof log,
  ): Promise<void> {
    const ip = clientIp(req, cfg.trustProxy);
    if (req.method === "POST" && req.url === "/api/auth/token") {
      if (!rateTakeToken(ip)) {
        requestLog.warn({ ip }, "rate limit exceeded on /api/auth/token");
        json(res, 429, { error: "rate_limited" });
        return;
      }
      // Real app: authenticate the caller here (session cookie, OAuth, etc).
      // See README "Deployment checklist" for the fences guarding this path.
      const { token, expiresAt } = await getKalamToken();
      json(res, 200, { token, expiresAt });
      return;
    }
    if (req.method === "GET" && req.url === "/api/health") {
      if (!rateTakeHealth(ip)) {
        json(res, 429, { error: "rate_limited" });
        return;
      }
      // Serve from cache if fresh.
      if (healthCache && Date.now() - healthCache.at < cfg.healthCacheMs) {
        json(res, healthCache.status, { ok: healthCache.ok, upstream: healthCache.upstream });
        return;
      }
      try {
        const r = await fetch(`${cfg.kalamdbUrl}/v1/api/health`, {
          signal: AbortSignal.timeout(2000),
        });
        if (!r.ok) {
          healthCache = { ok: false, status: 503, at: Date.now(), upstream: r.status };
          json(res, 503, { ok: false, upstream: r.status });
          return;
        }
      } catch (err) {
        healthCache = { ok: false, status: 503, at: Date.now(), upstream: "unreachable" };
        json(res, 503, { ok: false, upstream: "unreachable" });
        requestLog.warn({ err }, "upstream KalamDB health probe failed");
        return;
      }
      healthCache = { ok: true, status: 200, at: Date.now() };
      json(res, 200, { ok: true });
      return;
    }
    json(res, 404, { error: "not_found" });
  }

  return createServer((req, res) => {
    const incoming = req.headers["x-request-id"];
    const requestId =
      typeof incoming === "string" && /^[\w-]{8,128}$/.test(incoming) ? incoming : randomUUID();
    setSecurityHeaders(res, requestId);
    const requestLog = log.child({ request_id: requestId, method: req.method, url: req.url });
    const start = Date.now();
    handle(req, res, requestLog)
      .catch((err) => {
        requestLog.error({ err }, "handler error");
        if (!res.headersSent) json(res, 500, { error: "internal" });
        else res.end();
      })
      .finally(() => {
        requestLog.info({ status: res.statusCode, duration_ms: Date.now() - start }, "request");
      });
  });
}

// ---------------------------------------------------------------------------
// CLI entrypoint
// ---------------------------------------------------------------------------
//
// Tests import buildServer() directly with NODE_ENV=test, so we skip the
// auto-listen and the production fence in that mode.

function assertProductionFence(): void {
  if (process.env.NODE_ENV !== "production") return;
  if (process.env.ALLOW_UNAUTHENTICATED_TOKENS === "true") {
    log.warn(
      "/api/auth/token is mounted WITHOUT caller authentication (ALLOW_UNAUTHENTICATED_TOKENS=true). " +
        "Anyone reaching this endpoint can mint a KalamDB token. Only acceptable behind a private " +
        "network boundary or for an internal demo. See README for plugging in real auth.",
    );
    return;
  }
  const msg =
    "REFUSING TO START: NODE_ENV=production but /api/auth/token has no caller authentication. " +
    "Plug in real auth in server/index.ts (validate the caller's session and mint a per-user " +
    "KalamDB token), or set ALLOW_UNAUTHENTICATED_TOKENS=true to deliberately bypass this fence. " +
    "See README 'Deployment checklist'.";
  log.fatal(msg);
  throw new Error(msg);
}

if (process.env.NODE_ENV !== "test") {
  assertProductionFence();
  const port = Number(process.env.PORT ?? DEFAULTS.port);
  const server = buildServer();
  server.listen(port, "127.0.0.1", () => {
    log.info({ port }, "server listening");
  });
  function shutdown(signal: string): void {
    log.info({ signal }, "shutting down");
    server.close(() => process.exit(0));
    setTimeout(() => process.exit(0), 3000).unref();
  }
  process.on("SIGINT", () => shutdown("SIGINT"));
  process.on("SIGTERM", () => shutdown("SIGTERM"));
}
