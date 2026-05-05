# KalamDB TypeScript SDKs

This folder contains the publishable TypeScript SDK packages:

- `client/` for `@kalamdb/client`: auth, SQL, FILE columns, live queries, subscriptions, and typed cell values.
- `consumer/` for `@kalamdb/consumer`: topic polling, acknowledgements, and the agent/worker runtime.
- `orm/` for `@kalamdb/orm`: Drizzle ORM driver, KalamDB table helpers, FILE/BYTES/EMBEDDING columns, live table helpers, and schema generation.

Use each package directory as the source of truth for its build, test, and publish workflow. The packages are intentionally split so UI code can depend on `@kalamdb/client` and `@kalamdb/orm`, while worker processes add `@kalamdb/consumer` only when they need topic consumption.

## Developer handoff checklist

- Browser/admin UI: install `@kalamdb/client @kalamdb/orm drizzle-orm`.
- Topic workers/agents: install `@kalamdb/client @kalamdb/consumer`.
- Apps that share Drizzle schema between UI and workers can generate `schema.ts` with `kalamdb-orm`, then keep it fresh in local dev with `kalam --watch-schema --run "npm run schema:gen" --run-on-start`.
- `BIGINT` values are JSON-safe strings by default because KalamDB preserves Int64 precision on the wire.
- Exact KalamDB types are represented in the SDK: `BOOLEAN`, `INT`, `BIGINT`, `DOUBLE`, `FLOAT`, `TEXT`, `TIMESTAMP`, `DATE`, `DATETIME`, `TIME`, `JSON`, `BYTES`, `EMBEDDING(n)`, `UUID`, `DECIMAL(p,s)`, `SMALLINT`, and `FILE`.

## Common commands

```bash
cd link/sdks/typescript/client && npm run build:ts
cd link/sdks/typescript/orm && npm run build
cd link/sdks/typescript/consumer && npm run build:ts
```

Full package builds also compile/copy the package-specific WASM artifacts.
