# KalamDB TypeScript SDKs

This folder contains the publishable TypeScript SDK packages:

- `client/` for `@kalamdb/client`: auth, SQL, FILE columns, live queries, subscriptions, and typed cell values.
- `consumer/` for `@kalamdb/consumer`: topic polling, acknowledgements, and the agent/worker runtime.
- `orm/` for `@kalamdb/orm`: Drizzle ORM driver, KalamDB table helpers, FILE/BYTES/EMBEDDING columns, live table helpers, and schema generation.
- `react-old/` for `@kalamdb/react`: React provider, typed/raw live-query hooks, multi-query orchestration, mutation state, and component wrappers for KalamDB live queries.

Use each package directory as the source of truth for its build, test, and publish workflow. The packages are intentionally split so UI code can depend on `@kalamdb/client`, `@kalamdb/orm`, and `@kalamdb/react`, while worker processes add `@kalamdb/consumer` only when they need topic consumption.

## Developer handoff checklist

- Browser/admin UI: install `@kalamdb/client @kalamdb/react`; add `@kalamdb/orm drizzle-orm` for typed Drizzle mode, generated schemas, and assistant-style multi-query screens.
- Topic workers/agents: install `@kalamdb/client @kalamdb/consumer`.
- Apps that share Drizzle schema between UI and workers can generate `schema.ts` with `kalamdb-orm`, then keep it fresh in local dev with `kalam --watch-schema --run "npm run schema:gen" --run-on-start`.
- `BIGINT` values are JSON-safe strings by default because KalamDB preserves Int64 precision on the wire.
- Exact KalamDB types are represented in the SDK: `BOOLEAN`, `INT`, `BIGINT`, `DOUBLE`, `FLOAT`, `TEXT`, `TIMESTAMP`, `DATE`, `DATETIME`, `TIME`, `JSON`, `BYTES`, `EMBEDDING(n)`, `UUID`, `DECIMAL(p,s)`, `SMALLINT`, and `FILE`.

## Common commands

```bash
cd link/sdks/typescript/client && npm run build:ts
cd link/sdks/typescript/orm && npm run build
cd link/sdks/typescript/react-old && npm run build
cd link/sdks/typescript/consumer && npm run build:ts
```

Full package builds also compile/copy the package-specific WASM artifacts.

## React AI Chat Validation App

Use [../../../examples/react-ai-chat](../../../examples/react-ai-chat) to try the React SDK in a browser app with real UI composition: conversation sidebar, history loading, multi-file sends, typing, streamed assistant activity, tool calls, and human approvals.

```bash
cd examples/react-ai-chat
npm install
npm run setup
npm run dev
```

The app uses demo mode by default and can be switched to a server-backed KalamDB flow with `chat-app.sql` and `npm run agent`.

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See [../../../LICENSE.txt](../../../LICENSE.txt) and [../../../NOTICE](../../../NOTICE).
