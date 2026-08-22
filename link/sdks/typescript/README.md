# KalamDB TypeScript SDKs

This folder contains the publishable TypeScript SDK packages:

- `cli/` for `@kalamdb/cli`: lightweight npm wrapper that bootstraps the native `kalam` binary and delegates install/update logic to `kalam update`.
- `client/` for `@kalamdb/client`: auth, SQL, FILE columns, live queries, subscriptions, and typed cell values.
- `consumer/` for `@kalamdb/consumer`: topic polling, acknowledgements, and the agent/worker runtime.
- `orm/` for `@kalamdb/orm`: Drizzle ORM driver, KalamDB table helpers, FILE/BYTES/EMBEDDING columns, live table helpers, and schema generation.
- `react/` for `@kalamdb/react`: React provider, typed/raw live-query hooks, multi-query orchestration, mutation state, and component wrappers for KalamDB live queries.

Use each package directory as the source of truth for its build, test, and publish workflow. The packages are intentionally split so UI code can depend on `@kalamdb/client`, `@kalamdb/orm`, and `@kalamdb/react`, while worker processes add `@kalamdb/consumer` only when they need topic consumption.

## Maintainer package map

| Directory | Source package | npm publish name | GitHub Packages name |
| --- | --- | --- | --- |
| `cli/` | `@kalamdb/cli` | `@kalamdb/cli` | `@kalamdb/cli` |
| `client/` | `@kalamdb/client` | `@kalamdb/client` | `@kalamdb/client` |
| `consumer/` | `@kalamdb/consumer` | `@kalamdb/consumer` | `@kalamdb/consumer` |
| `orm/` | `@kalamdb/orm` | `@kalamdb/orm` | `@kalamdb/orm` |
| `react/` | `@kalamdb/react` | `@kalamdb/react` | `@kalamdb/react` |

The source manifests stay on `@kalamdb/*`. The GitHub Packages publish lane stages temporary `@kalamdb/*` manifests during publish because GitHub Packages requires the package scope to match the repository owner namespace.

## Version maintenance

The root Cargo workspace version in `Cargo.toml` is the shared version anchor for the publishable SDK cohort across TypeScript, Dart, and Python. After changing `[workspace.package].version`, run:

```bash
bash link/sdks/sync-versions.sh
```

That script:

- sets all five TypeScript package `version` fields to the root Cargo workspace version,
- updates all `link/sdks/dart/*/pubspec.yaml` package versions,
- updates `link/sdks/python/pyproject.toml` and `link/sdks/python/Cargo.toml` to the same version,
- updates the internal peer dependency floors to the current cohort range,
- regenerates `versions.json` through `python3 scripts/versions.py sync --write`, and
- verifies the generated manifest with `python3 scripts/versions.py verify`.

Internal peer dependency ranges follow the shared cohort. For prerelease lanes like `0.5.0-beta.1`, the sync script keeps prerelease-safe floors such as `>=0.5.0-0 <0.6.0`.

## Developer handoff checklist

- Browser/admin UI: install `@kalamdb/client @kalamdb/react`; add `@kalamdb/orm drizzle-orm` for typed Drizzle mode, generated schemas, and assistant-style multi-query screens.
- Topic workers/agents: install `@kalamdb/client @kalamdb/consumer`.
- Apps that share Drizzle schema between UI and workers can generate `schema.ts` with `kalamdb-orm`, then keep it fresh in local dev with `kalam --watch-schema --run "npm run schema:gen" --run-on-start`.
- `BIGINT` values are JSON-safe strings by default because KalamDB preserves Int64 precision on the wire.
- Exact KalamDB types are represented in the SDK: `BOOLEAN`, `INT`, `BIGINT`, `DOUBLE`, `FLOAT`, `TEXT`, `TIMESTAMP`, `DATE`, `DATETIME`, `TIME`, `JSON`, `BYTES`, `EMBEDDING(n)`, `UUID`, `DECIMAL(p,s)`, `SMALLINT`, and `FILE`.

## Build and test

The release-lane test script in `scripts/test-typescript-sdk-release.sh` is the maintainer-facing source of truth. By default it runs all five packages: `client consumer orm react cli`.

```bash
./scripts/test-typescript-sdk-release.sh
TS_SDK_PACKAGES="client react cli" ./scripts/test-typescript-sdk-release.sh
python3 scripts/versions.py verify
```

For short local loops you can still run package-local commands:

```bash
cd link/sdks/typescript/cli && npm test
cd link/sdks/typescript/client && npm run build:ts
cd link/sdks/typescript/orm && npm run build
cd link/sdks/typescript/react && npm run build
cd link/sdks/typescript/consumer && npm run build:ts
```

Full package builds also compile/copy the package-specific WASM artifacts.

## Publishing and release checks

The GitHub Actions workflow `.github/workflows/typescript-sdk.yml` is **manual-only** (`workflow_dispatch`). It runs the shared TypeScript package test matrix and optional npm publishing for the app/runtime SDKs.

- Manual input `publish=true` publishes `@kalamdb/client`, `@kalamdb/consumer`, `@kalamdb/orm`, and `@kalamdb/react` to npm. Each package `publish.sh` skips when that version already exists.
- Manual input `publish_github_packages=true` publishes the GitHub Packages variants for the same four packages.
- Manual input `force_publish=true` asks each package `publish.sh` to attempt an unpublish and republish when the registry allows it.
- The shared test matrix runs `client`, `consumer`, `orm`, `react`, and `cli`.
- Publish order: `client` → `consumer` → `orm` → `react`.
- `@kalamdb/cli` publishes from `.github/workflows/release.yml` when the **Publish @kalamdb/cli to npm** or **Publish @kalamdb/cli to GitHub Packages** checkbox is enabled (also skips existing versions).

Expected registry secrets and tokens:

- npm publish uses `NPM_TOKEN` in GitHub Actions.
- GitHub Packages publish uses `GH_PACKAGES_TOKEN` when provided, otherwise the workflow falls back to `github.token`.

If you need to inspect publish behavior locally, each package directory has a `publish.sh` entrypoint. The workflow is still the preferred release path because it runs the shared test matrix and publishes in dependency order.

## React AI Chat Validation App

Use `examples/react-ai-chat` to try the React SDK in a browser app with real UI composition: conversation sidebar, history loading, multi-file sends, typing, streamed assistant activity, tool calls, and human approvals.

```bash
cd examples/react-ai-chat
npm install
npm run setup
npm run dev
```

The app uses demo mode by default and can be switched to a server-backed KalamDB flow with `chat-app.sql` and `npm run agent`.

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See the packaged `LICENSE.txt` and `NOTICE` files in each SDK package.
