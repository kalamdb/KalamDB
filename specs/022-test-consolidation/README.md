# Test Consolidation Spec (Backend/CLI)

Date: 2026-01-22
Owner: KalamDB

## Goals
- Consolidate **all backend integration tests** under `backend/tests`.
- Support three runtime modes for backend tests:
  - `test`: in-process HTTP testserver (current `backend/tests/common/testserver`).
  - `single`: external single-node server (URL provided via `--urls`).
  - `cluster`: external 3-node cluster (URLs provided via `--urls`).
- Use **kalam-link models** for response parsing in all backend tests.
- Keep **CLI-only tests** in `cli/tests` (CLI UX + config + credential-store behaviors).
- Move **no-server unit tests** into their owning `kalamdb-*` crates.
- Provide a **simple API** for tests (e.g., `let server = TestServer::instance().await;`).
- Support **explicit password** input for remote/docker environments.
- Allow **skipping tests by runtime** (e.g., cluster-only tests are ignored when runtime is `single`).
- Ensure testserver HTTP calls use **JWT auth** cached in the singleton (no per-call basic auth).

## Non-Goals
- Changing server behavior or query semantics.
- Rewriting business logic in tests (only wiring/organization changes).
- Removing existing test coverage.

## Current State (Observed)
- Runtime selection/URL logic currently lives in `cli/tests/common/mod.rs` and includes:
  - `--urls`, leader detection, cluster retry logic, kalam-link client helpers.
- Backend tests already have in-process HTTP testserver + cluster helpers in:
  - `backend/tests/common/testserver`.
- CLI tests include both CLI-only and backend integration tests.
- Some CLI tests are unit tests for kalam-link or CLI credential storage and don’t require a server.

## Target Structure
```
backend/tests/
  common/
    runtime.rs            # NEW: runtime selection + URL resolution + client factory
    link_helpers.rs       # NEW or merged into runtime.rs
    testserver/           # Existing: in-process HTTP + cluster server
  ... existing test categories ...

cli/tests/
  cli/                    # CLI-only integration tests
  common/                 # CLI-only helpers (no backend runtime logic)

backend/crates/kalamdb-link/   # (actual crate name: link) unit tests
backend/crates/kalamdb-cli/    # (actual crate name: cli) unit tests
```

## Runtime Interface (backend/tests/common/runtime.rs)
### CLI Args
- `--runtime <test|single|cluster>`
- `--urls <comma-delimited>`
- Optional: `--user`, `--password` (with env fallbacks)

### Default Resolution Rules
1. If `--runtime` is supplied, use it.
2. If `--runtime` is omitted:
   - `--urls` with 1 URL → `single`
   - `--urls` with >=2 URLs → `cluster`
   - no `--urls` → `test`

### Runtime Types
```rust
enum RuntimeMode { Test, Single, Cluster }

struct RuntimeConfig {
    mode: RuntimeMode,
    urls: Vec<String>,      // resolved URLs
    username: String,
    password: String,
}

struct TestRuntime {
    config: RuntimeConfig,
}
```

### TestServer API (simple)
- Provide a simple entry point for tests:
  - `let server = TestServer::instance().await;`
- `TestServer::instance()` returns a runtime-aware singleton wrapper.
- If a test requires direct `AppContext` access and runtime is not `test`, the test must skip.

### Required Functions
- `TestRuntime::from_args_env() -> Self`
- `TestRuntime::mode() -> RuntimeMode`
- `TestRuntime::urls() -> &[String]`
- `TestRuntime::leader_url() -> Option<String>` (cluster only)
- `TestRuntime::client_root() -> KalamLinkClient`
- `TestRuntime::client_for_user(user, pass) -> KalamLinkClient`
- `TestRuntime::require_cluster()` (skip/panic when cluster-only tests run in other modes)

### Behavior
- `runtime=test` uses in-process `HttpTestServer` and builds a kalam-link client via its helpers (or a new bridge), so all SQL responses are parsed via kalam-link models.
- `runtime=single` uses the first URL from `--urls`.
- `runtime=cluster` uses all URLs from `--urls` and prioritizes leader for writes.
- JWT auth is fetched once per runtime and cached in the singleton (no per-call basic auth). If a password is provided, use it to login and store the token.

## Migration Plan (High Level)
### 1) Create runtime helpers in backend/tests/common
- Move/port these helpers from `cli/tests/common/mod.rs`:
  - URL parsing (`--urls`), leader detection and caching.
  - cluster retry logic.
  - client helpers for SQL execution and response parsing.
- Replace raw JSON parsing with kalam-link `QueryResponse` / `QueryResult`.

### 2) Move backend integration tests from cli/tests → backend/tests
Move into appropriate backend categories (auth, tables, storage, flushing, subscription, cluster, smoke, usecases):
- `cli/tests/auth/*`
- `cli/tests/users/*`
- `cli/tests/tables/*`
- `cli/tests/storage/*`
- `cli/tests/flushing/*`
- `cli/tests/subscription/*` (server-required tests)
- `cli/tests/cluster/*`
- `cli/tests/usecases/*`
- `cli/tests/smoke/*`
- `cli/tests/connection/live_connection_tests.rs`

### 3) Keep CLI-only tests in cli/tests
- `cli/tests/cli/*` (help/version/config/terminal UX)
- CLI config and credential store validation

### 4) Move unit tests into crates (no server required)
**kalam-link crate (link/):**
- `cli/tests/connection/connection_options_tests.rs`
- `cli/tests/connection/reconnection_tests.rs`
- `cli/tests/connection/resume_seqid_tests.rs`
- `cli/tests/connection/timeout_tests.rs`
- `cli/tests/subscription/subscription_options_tests.rs`

**kalam-cli crate (cli/):**
- Unit tests in `cli/tests/cli/test_cli_auth.rs` labeled “UNIT TESTS - FileCredentialStore”.

## Test Execution Matrix
| Mode     | Command Example |
|----------|-----------------|
| test     | `cargo test -p kalamdb-server --test <name> -- --runtime test` |
| single   | `cargo test -p kalamdb-server --test <name> -- --runtime single --urls http://localhost:2900` |
| cluster  | `cargo test -p kalamdb-server --test <name> -- --runtime cluster --urls http://localhost:2901,http://localhost:2902,http://localhost:2903` |

Notes:
- `--urls` is **required** for `single` and `cluster`.
- Cluster-only tests must gate on `runtime=cluster`.
- Tests requiring `AppContext` or any specific service/factory from the backend code must be `runtime=test` and should skip otherwise.

## Validation Checklist
- [ ] All backend integration tests compile under `backend/tests`.
- [ ] All backend tests execute with `--runtime test` using in-process server.
- [ ] All backend tests execute with `--runtime single` using external server.
- [ ] All backend tests execute with `--runtime cluster` using external cluster.
- [ ] All SQL parsing in backend tests uses kalam-link models.
- [ ] CLI tests are strictly CLI-focused (no backend feature validation).
- [ ] No-server unit tests moved into their crates.

## Risks & Mitigations
- **Leader detection errors** in cluster mode: keep leader caching and retry logic from existing CLI test helpers.
- **Shared test server state** for `runtime=test`: enforce unique namespaces/tables in tests.
- **Auth differences** in external runtime: provide `--user/--password` args and env fallbacks.
- **JWT cache invalidation**: if login fails or token expires, refresh once and retry.

## Implementation Notes
- `backend/tests/common/testserver` stays as the in-process server layer.
- Prefer a single runtime helper module used by all backend tests to avoid divergence.
- Avoid importing modules inside functions; keep all `use` at top of files.
- Use type-safe wrappers and kalam-link models as in existing backend tests.
