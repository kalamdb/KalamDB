# Server Lifecycle Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Give KalamDB one testable startup and shutdown implementation that owns transport readiness, background tasks, HTTP construction, and cleanup on every exit path.

**Architecture:** Keep storage and database-session ownership in their existing crates. Deepen `kalamdb-jobs` so its runtime owns and drains its task, deepen `kalamdb-postgres-wire` so startup returns only after the real listener is ready, and move HTTP construction plus shutdown coordination out of the broad lifecycle module. Keep `main.rs` responsible only for process-level setup and delegate KalamDB startup policy to the library.

**Tech Stack:** Rust 2024, Tokio, Actix Web, PostgreSQL wire, anyhow, cargo nextest.

---

### Task 1: Give the jobs loop an owned graceful-shutdown path

**Files:**
- Create: `backend/crates/kalamdb-jobs/src/jobs_manager/runtime.rs`
- Modify: `backend/crates/kalamdb-jobs/src/jobs_manager/mod.rs`
- Modify: `backend/crates/kalamdb-jobs/src/jobs_manager/runner.rs`
- Modify: `backend/crates/kalamdb-jobs/src/lib.rs`

**Step 1: Write the failing tests**

Add async tests specifying that shutdown waits for an in-flight task to finish, reports a timeout for a non-cooperative task, and preserves a run-loop error instead of discarding it.

**Step 2: Run the focused test to verify it fails**

Run: `cargo nextest run -p kalamdb-jobs runtime --no-fail-fast`

Expected: FAIL because the owned jobs runtime and drain result do not exist.

**Step 3: Write the minimal implementation**

Add a jobs runtime that owns `Arc<JobsManager>` and its `JoinHandle`, requests shutdown, waits under the caller's deadline, and aborts only after timeout. Change the run loop to drain its `JoinSet` cooperatively after it stops scheduling new work instead of immediately aborting every task.

**Step 4: Run the focused tests and crate check**

Run: `cargo nextest run -p kalamdb-jobs runtime --no-fail-fast`

Run: `cargo check -p kalamdb-jobs`

Expected: PASS.

**Step 5: Review checkpoint**

Review only the jobs diff. Do not commit because the working tree already contains user-owned changes that overlap this refactor.

### Task 2: Make PostgreSQL-wire readiness and termination observable

**Files:**
- Modify: `backend/crates/kalamdb-postgres-wire/src/listener.rs`
- Modify: `backend/crates/kalamdb-postgres-wire/src/server.rs`
- Modify: `backend/crates/kalamdb-postgres-wire/src/lib.rs`

**Step 1: Write the failing tests**

Add tests specifying that startup fails when the configured port is occupied, TLS validation fails before a background task is detached, and listener task errors are returned to the owner.

**Step 2: Run the focused test to verify it fails**

Run: `cargo nextest run -p kalamdb-postgres-wire listener --no-fail-fast`

Expected: FAIL because `PostgresWireListener::start` does not return readiness errors.

**Step 3: Write the minimal implementation**

Bind the real Tokio listener and load TLS before spawning. Store a `JoinHandle<std::io::Result<()>>`; expose owner-only waiting and shutdown operations that preserve both task-join and listener errors.

**Step 4: Run the focused tests and crate check**

Run: `cargo nextest run -p kalamdb-postgres-wire listener --no-fail-fast`

Run: `cargo check -p kalamdb-postgres-wire`

Expected: PASS.

**Step 5: Review checkpoint**

Confirm the existing uncommitted PostgreSQL handler/session extraction remains intact and no transport-specific transaction rules moved out of `BackendSessionManager`.

### Task 3: Deepen HTTP construction into one module

**Files:**
- Create: `backend/src/http_server.rs`
- Modify: `backend/src/http_runtime.rs`
- Modify: `backend/src/lifecycle.rs`
- Modify: `backend/src/lib.rs`

**Step 1: Write the failing tests**

Add tests for effective worker settings, explicit-listener binding, protocol selection, and propagation of nested Actix server errors through the owned server handle.

**Step 2: Run the focused test to verify it fails**

Run: `cargo nextest run -p kalamdb-server --lib http_server --no-fail-fast`

Expected: FAIL because the HTTP server runtime module does not exist.

**Step 3: Write the minimal implementation**

Move middleware, application data, route/UI registration, and common Actix tuning into one internal implementation. Bind a real `std::net::TcpListener` before starting other transports and use `listen` or `listen_auto_h2c`. Keep production, fixed-address detached, and ephemeral test binding as small policy choices.

**Step 4: Run focused tests and server library check**

Run: `cargo nextest run -p kalamdb-server --lib http_server --no-fail-fast`

Run: `cargo check -p kalamdb-server`

Expected: PASS.

### Task 4: Route every termination reason through one shutdown implementation

**Files:**
- Create: `backend/src/shutdown.rs`
- Modify: `backend/src/lifecycle.rs`
- Modify: `backend/src/http_server.rs`
- Modify: `backend/tests/misc/production/test_graceful_shutdown.rs`

**Step 1: Write the failing tests**

Specify typed termination reasons for Ctrl+C, SIGTERM, HTTP completion/failure, PostgreSQL completion/failure, and explicit test shutdown. Add state-based tests showing that ingress stops before connection draining, jobs are drained under the configured deadline, all owned tasks are joined, and the original failure is returned after cleanup.

**Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p kalamdb-server --lib shutdown --no-fail-fast`

Expected: FAIL because termination reasons and unified cleanup do not exist.

**Step 3: Write the minimal implementation**

Centralize termination selection and cleanup. Stop HTTP gracefully before draining WebSocket and PostgreSQL connections; shut down the owned jobs runtime with the flush deadline; always ask the executor to shut down; release application ownership; and return HTTP/PostgreSQL/task errors after cleanup. Production signals and test handles only request termination.

**Step 4: Run focused unit and process tests**

Run: `cargo nextest run -p kalamdb-server --lib shutdown --no-fail-fast`

Run: `cargo nextest run -p kalamdb-server --test test_misc kalamdb_server_gracefully_shuts_down_on_sigterm --no-fail-fast`

Expected: PASS with clean SIGTERM exit and shutdown completion logs.

### Task 5: Reduce `main.rs` to the process shell and make startup cleanup-safe

**Files:**
- Create: `backend/src/process.rs`
- Modify: `backend/src/startup.rs`
- Modify: `backend/src/logging.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/src/lib.rs`

**Step 1: Write the failing tests**

Move command parsing, configuration/security validation, runtime sizing, and partial-startup behavior into library-visible tests. Specify that telemetry is shut down after bootstrap or run failure and that startup does not announce readiness until all enabled transports are bound.

**Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p kalamdb-server --lib process --no-fail-fast`

Expected: FAIL because process startup policy remains in the binary.

**Step 3: Write the minimal implementation**

Keep allocator installation, file-descriptor policy, and the top-level call in `main.rs`. Move testable command/config/runtime/security startup logic to the library. Use scoped cleanup so telemetry and any bootstrapped KalamDB resources are cleaned up on every returned error.

**Step 4: Run focused tests and server check**

Run: `cargo nextest run -p kalamdb-server --lib process --no-fail-fast`

Run: `cargo check -p kalamdb-server`

Expected: PASS.

### Task 6: Simplify, format, and verify the complete refactor

**Files:**
- Modify only the files changed in Tasks 1-5.

**Step 1: Apply the deletion test**

Remove pass-through helpers and duplicated Actix configuration. Keep only real seams with multiple modes or meaningful behavior.

**Step 2: Format and inspect**

Run: `cargo fmt --all -- --check`

Run: `git diff --check`

Expected: PASS.

**Step 3: Run focused verification**

Run: `cargo check -p kalamdb-server`

Run: `cargo nextest run -p kalamdb-jobs -p kalamdb-postgres-wire -p kalamdb-server --lib --no-fail-fast`

Run: `cargo nextest run -p kalamdb-server --test test_misc kalamdb_server_gracefully_shuts_down_on_sigterm --no-fail-fast`

Expected: PASS.

**Step 4: Run backend/CLI behavior smoke coverage if the server behavior changed materially**

Start the backend with a temporary test configuration, build the CLI, and run the relevant smoke suite as required by `AGENTS.md`.

**Step 5: Final diff audit**

Confirm unrelated RLS, benchmark, SDK, and test-log changes were neither staged nor modified by this work.
