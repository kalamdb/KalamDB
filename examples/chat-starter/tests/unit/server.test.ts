import { test } from "node:test";
import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { buildServer, type ServerConfig } from "../../server/index.js";

const baseConfig: ServerConfig = {
  kalamdbUrl: "http://kalamdb.invalid",
  kalamdbUser: "root",
  kalamdbPassword: "x",
  tokenRateLimitPerMinute: 60,
  healthRateLimitPerMinute: 600,
  healthCacheMs: 0, // disable cache in most tests
  trustProxy: false,
};

interface RunningServer {
  url: string;
  close: () => Promise<void>;
}

async function withServer(
  opts: Parameters<typeof buildServer>[0],
  fn: (s: RunningServer) => Promise<void>,
): Promise<void> {
  const server = buildServer(opts);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addr = server.address() as AddressInfo;
  const handle: RunningServer = {
    url: `http://127.0.0.1:${addr.port}`,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
  try {
    await fn(handle);
  } finally {
    await handle.close();
  }
}

test("POST /api/auth/token returns the cached KalamDB token", async () => {
  let fetched = 0;
  await withServer(
    {
      config: baseConfig,
      tokenFetcher: async () => {
        fetched += 1;
        return { token: "kalam-token-xyz", expiresAt: Date.now() + 60_000 };
      },
    },
    async (s) => {
      const res = await fetch(`${s.url}/api/auth/token`, { method: "POST" });
      assert.equal(res.status, 200);
      const body = (await res.json()) as { token: string; expiresAt: number };
      assert.equal(body.token, "kalam-token-xyz");
      assert.equal(typeof body.expiresAt, "number");
      // Security + correlation headers must be present.
      assert.equal(res.headers.get("x-content-type-options"), "nosniff");
      assert.equal(res.headers.get("x-frame-options"), "DENY");
      assert.equal(res.headers.get("referrer-policy"), "strict-origin-when-cross-origin");
      assert.match(res.headers.get("x-request-id") ?? "", /^[0-9a-f-]{36}$/);
      assert.equal(fetched, 1);
    },
  );
});

test("POST /api/auth/token surfaces 500 when upstream fetcher throws", async () => {
  await withServer(
    {
      config: baseConfig,
      tokenFetcher: async () => {
        throw new Error("KalamDB login failed (401): bad creds");
      },
    },
    async (s) => {
      const res = await fetch(`${s.url}/api/auth/token`, { method: "POST" });
      assert.equal(res.status, 500);
      const body = (await res.json()) as { error: string };
      assert.equal(body.error, "internal");
    },
  );
});

test("POST /api/auth/token enforces per-IP rate limit", async () => {
  const rateLimit = 3;
  await withServer(
    {
      config: { ...baseConfig, tokenRateLimitPerMinute: rateLimit },
      tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
    },
    async (s) => {
      for (let i = 0; i < rateLimit; i++) {
        const res = await fetch(`${s.url}/api/auth/token`, { method: "POST" });
        assert.equal(res.status, 200, `request ${i + 1} should succeed`);
      }
      const limited = await fetch(`${s.url}/api/auth/token`, { method: "POST" });
      assert.equal(limited.status, 429);
      const body = (await limited.json()) as { error: string };
      assert.equal(body.error, "rate_limited");
    },
  );
});

test("X-Request-ID is echoed when supplied by the caller", async () => {
  await withServer(
    {
      config: baseConfig,
      tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
    },
    async (s) => {
      const supplied = "11111111-2222-3333-4444-555555555555";
      const res = await fetch(`${s.url}/api/auth/token`, {
        method: "POST",
        headers: { "x-request-id": supplied },
      });
      assert.equal(res.headers.get("x-request-id"), supplied);
    },
  );
});

test("GET /api/health probes upstream and reports 503 on failure", async () => {
  // Save and restore globalThis.fetch so we don't leak the mock.
  const realFetch = globalThis.fetch;
  globalThis.fetch = ((_input: unknown, _init?: unknown) => {
    return Promise.reject(new Error("upstream down"));
  }) as typeof fetch;
  try {
    await withServer(
      {
        config: baseConfig,
        tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
      },
      async (s) => {
        const res = await realFetch(`${s.url}/api/health`);
        assert.equal(res.status, 503);
      },
    );
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("GET /api/health returns 200 when upstream is OK", async () => {
  const realFetch = globalThis.fetch;
  globalThis.fetch = ((input: unknown, _init?: unknown) => {
    const url = typeof input === "string" ? input : (input as { url: string }).url;
    if (url.includes("/v1/api/health")) {
      return Promise.resolve(new Response("{}", { status: 200 }));
    }
    return realFetch(input as string | URL | Request);
  }) as typeof fetch;
  try {
    await withServer(
      {
        config: baseConfig,
        tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
      },
      async (s) => {
        const res = await realFetch(`${s.url}/api/health`);
        assert.equal(res.status, 200);
      },
    );
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("unknown routes return 404", async () => {
  await withServer(
    {
      config: baseConfig,
      tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
    },
    async (s) => {
      const res = await fetch(`${s.url}/api/nope`);
      assert.equal(res.status, 404);
    },
  );
});

test("X-Forwarded-For is IGNORED when trustProxy is false (anti-spoofing)", async () => {
  const rateLimit = 2;
  await withServer(
    {
      config: { ...baseConfig, tokenRateLimitPerMinute: rateLimit, trustProxy: false },
      tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
    },
    async (s) => {
      // Hit the limit using rotating X-Forwarded-For values. Because
      // trustProxy=false, all requests count toward the same socket IP and
      // the 3rd one must be rate-limited.
      const r1 = await fetch(`${s.url}/api/auth/token`, {
        method: "POST",
        headers: { "x-forwarded-for": "1.1.1.1" },
      });
      const r2 = await fetch(`${s.url}/api/auth/token`, {
        method: "POST",
        headers: { "x-forwarded-for": "2.2.2.2" },
      });
      const r3 = await fetch(`${s.url}/api/auth/token`, {
        method: "POST",
        headers: { "x-forwarded-for": "3.3.3.3" },
      });
      assert.equal(r1.status, 200);
      assert.equal(r2.status, 200);
      assert.equal(r3.status, 429, "spoofed XFF must not bypass the per-IP limit");
    },
  );
});

test("X-Forwarded-For IS honored when trustProxy is true", async () => {
  const rateLimit = 1;
  await withServer(
    {
      config: { ...baseConfig, tokenRateLimitPerMinute: rateLimit, trustProxy: true },
      tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
    },
    async (s) => {
      // Two requests from "different" XFF IPs both succeed because trustProxy
      // is true and each gets its own bucket.
      const r1 = await fetch(`${s.url}/api/auth/token`, {
        method: "POST",
        headers: { "x-forwarded-for": "10.0.0.1" },
      });
      const r2 = await fetch(`${s.url}/api/auth/token`, {
        method: "POST",
        headers: { "x-forwarded-for": "10.0.0.2" },
      });
      assert.equal(r1.status, 200);
      assert.equal(r2.status, 200);
    },
  );
});

test("/api/health serves cached answer within healthCacheMs window", async () => {
  let probeCount = 0;
  const realFetch = globalThis.fetch;
  globalThis.fetch = ((input: unknown, _init?: unknown) => {
    const url = typeof input === "string" ? input : (input as { url: string }).url;
    if (url.includes("/v1/api/health")) {
      probeCount++;
      return Promise.resolve(new Response("{}", { status: 200 }));
    }
    return realFetch(input as string | URL | Request);
  }) as typeof fetch;
  try {
    await withServer(
      {
        config: { ...baseConfig, healthCacheMs: 10_000 },
        tokenFetcher: async () => ({ token: "t", expiresAt: Date.now() + 60_000 }),
      },
      async (s) => {
        for (let i = 0; i < 5; i++) {
          const r = await realFetch(`${s.url}/api/health`);
          assert.equal(r.status, 200);
        }
        assert.equal(probeCount, 1, "upstream must be probed exactly once when cache is fresh");
      },
    );
  } finally {
    globalThis.fetch = realFetch;
  }
});
