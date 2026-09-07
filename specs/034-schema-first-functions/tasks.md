# Tasks: Schema-First Functions Runtime (V1.1)

**Input**: Design documents from `/specs/034-schema-first-functions/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Required by the spec (security, pool isolation, revision pin, inline override, CLI e2e, Stage 0/8 benches). Stability-first: capture baseline, change narrowly, run the story gate. Write failing tests before implementation where a story lists an Independent Test.

**Organization**: Setup → Foundational → user stories in plan Stage order (not numeric US order). Each story stays independently testable after its checkpoint.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Different files, no dependency on an incomplete task
- **[Story]**: Maps to spec user story (US1–US24)
- Every task includes exact file paths

## Path Conventions

Workspace paths under `backend/crates/`, `cli/src/`, `benchv2/`, `docs/`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Record the PR #380 baseline and add shared deps/config before behavior changes.

- [x] T001 Capture Stage 0 baseline (existing nextest, `kalamdb_functions` bench runtimes in seconds, idle/warm RSS, `kalamdb-server` binary size) in `specs/034-schema-first-functions/validation/stage0-baseline.md` using commands in `specs/034-schema-first-functions/quickstart.md`
- [x] T002 Add workspace deps `arc-swap` and `smallvec` in root `Cargo.toml` and depend on them from `backend/crates/kalamdb-core/Cargo.toml` and `backend/crates/kalamdb-functions/Cargo.toml`
- [x] T003 Remove `wasm-runtime` from default features in `backend/crates/kalamdb-functions/Cargo.toml` (keep `default = ["catalog"]`; `wasm-runtime` optional) and fix feature-gated compile in `backend/crates/kalamdb-functions/src/lib.rs` and `backend/crates/kalamdb-functions/src/engine.rs`
- [x] T004 [P] Add `[functions.runtime]` server config (workers, max_active, max_memory_mb, heap_soft/hard, idle pool, timeout, host-op limits) in `backend/crates/kalamdb-configs/src/config/types.rs`, `backend/crates/kalamdb-configs/src/config/defaults.rs`, `backend/server.example.toml`, and `backend/crates/kalamdb-functions/src/engine_config.rs`
- [x] T005 [P] Add project `[functions] path/runtime/module` (default module `backend`) in `cli/src/workflow/project/config.rs` and document keys in `specs/034-schema-first-functions/contracts/runtime-config.md`

**Checkpoint**: Baseline numbers exist; Wasmtime is not in the default functions crate; config types compile.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared types, ABI constant, catalog columns, and runtime trait. No pool rewrite or CLI build yet.

**⚠️ CRITICAL**: User-story implementation waits until this phase compiles.

- [x] T006 Add typed function error codes (`PROCEDURE_NOT_FOUND`, `PROCEDURE_NOT_IMPLEMENTED`, `EXECUTE_DENIED`, `AUTHENTICATION_REQUIRED`, `INVALID_ARGUMENTS`, `RESOURCE_LIMIT`, `PROCEDURE_TIMEOUT`, `INTERNAL_RUNTIME_ERROR`, `CONTRACT_MISMATCH`, `ABI_MISMATCH`, `STALE_REVISION`) on `FunctionsError` in `backend/crates/kalamdb-functions/src/error.rs` and map them in `backend/crates/kalamdb-core/src/error.rs` per `specs/034-schema-first-functions/contracts/typed-errors.md`
- [x] T007 Set production `ABI_VERSION = 2` in `backend/crates/kalamdb-functions/src/limits.rs` and `backend/crates/kalamdb-functions/src/revision.rs` (keep v1 invoke path compiling until US8 removes it)
- [x] T008 Add `RoutineGrantee::Anonymous` in `backend/crates/kalamdb-commons` grant models and persist it in `backend/crates/kalamdb-system/src/providers/catalog/models/routines/catalog_routine_grant.rs`
- [x] T009 Add `inline_source_hash` and `inline_artifact_id` columns on `CatalogRoutine` in `backend/crates/kalamdb-system/src/providers/catalog/models/routines/catalog_routine.rs` and update catalog tests in `backend/crates/kalamdb-system/src/providers/catalog/catalog_tests.rs`
- [x] T010 Implement `ActiveFunctionSet`, `ImplementationRef`, and `ProcedureSlot` in `backend/crates/kalamdb-core/src/functions/active_set.rs` and export from `backend/crates/kalamdb-core/src/functions/mod.rs`
- [x] T011 Hold `Arc<arc_swap::ArcSwap<ActiveFunctionSet>>` on `FunctionRuntimeState` in `backend/crates/kalamdb-core/src/functions/runtime_state.rs`
- [x] T012 Add minimal `FunctionRuntime` trait (load module, invoke root, terminate) in `backend/crates/kalamdb-functions/src/runtime.rs` so `kalamdb-core` does not import `V8Session`; export from `backend/crates/kalamdb-functions/src/lib.rs`
- [x] T013 Wire `EngineConfig` from server `[functions.runtime]` into `FunctionRuntimeState::engine()` in `backend/crates/kalamdb-core/src/functions/runtime_state.rs` (still lazy `OnceLock`)
- [x] T014 Run `cargo check -p kalamdb-functions -p kalamdb-core -p kalamdb-system -p kalamdb-configs` and record output in `specs/034-schema-first-functions/validation/foundational-check.md`

**Checkpoint**: Types, catalog fields, and config exist; production invoke behavior unchanged except default features/config plumbing.

---

## Phase 3: User Story 1 - Declare Procedures as SQL Contracts (Priority: P1) 🎯 MVP

**Goal**: `CREATE PROCEDURE` is a SQL contract: signature, security, grants; no source-file mapping; no `LANGUAGE` without a body.

**Independent Test**: Create `SECURITY DEFINER` procedure with GRANT/REVOKE and no `AS 'src/...'`; missing implementation returns `PROCEDURE_NOT_IMPLEMENTED`.

### Tests

- [x] T015 [P] [US1] Add parser tests rejecting `AS 'src/api/orders.ts', 'createOrder'` and requiring LANGUAGE iff body exists in `backend/crates/kalamdb-dialect/src/ddl/create_procedure.rs`
- [x] T016 [P] [US1] Add handler/catalog test that a bodyless procedure stores a contract and CALL yields not-implemented in `backend/crates/kalamdb-handlers/crates/ddl/src/procedure/create.rs` tests (or core function tests)

### Implementation

- [x] T017 [US1] Reject source-file mapping and enforce LANGUAGE-only-with-body in `backend/crates/kalamdb-dialect/src/ddl/create_procedure.rs` per `specs/034-schema-first-functions/contracts/sql-create-procedure.md`
- [x] T018 [US1] Persist bodyless project-backed procedures without creating a per-routine function module in `backend/crates/kalamdb-handlers/crates/ddl/src/procedure/create.rs`
- [x] T019 [US1] Return typed `PROCEDURE_NOT_IMPLEMENTED` when `ImplementationRef::Missing` in `backend/crates/kalamdb-core/src/functions/executor.rs`

**Checkpoint**: Schema-first CREATE works; missing impl is a stable error.

---

## Phase 4: User Story 4 - Write Small Procedures Inline in SQL (Priority: P1)

**Goal**: Inline JS is executable; inline TS is stored and not executed by the server until compiled JS exists.

**Independent Test**: `LANGUAGE JAVASCRIPT` `api.health` CALLs `"ok"`; `LANGUAGE TYPESCRIPT` stores source and refuses execution until a compiled artifact exists.

### Tests

- [x] T020 [P] [US4] Add dialect/handler tests for dollar-quoted JS/TS bodies and hash persistence in `backend/crates/kalamdb-dialect/src/ddl/create_procedure.rs` and `backend/crates/kalamdb-system/src/providers/catalog/catalog_tests.rs`

### Implementation

- [x] T021 [US4] On inline CREATE, set `language`, `body`, `inline_source_hash`, and compiled `inline_artifact_id` only for JavaScript in `backend/crates/kalamdb-handlers/crates/ddl/src/procedure/create.rs`
- [x] T022 [US4] Compile inline JS to a content-addressed artifact via `backend/crates/kalamdb-functions/src/wrap.rs` + filestore; do not compile TypeScript on the server
- [x] T023 [US4] Refuse CALL of inline TypeScript without compiled JS with a clear diagnostic in `backend/crates/kalamdb-core/src/functions/executor.rs`

**Checkpoint**: Inline JS runs; inline TS is catalog-only until a project build.

---

## Phase 5: User Story 6 - Deploy One Module Containing Many Procedures (Priority: P1)

**Goal**: One `FunctionModule` (`backend`) and one revision/artifact for the whole project; stop `module_id = routine_id`.

**Independent Test**: Many procedures share a single module revision row.

### Tests

- [x] T024 [P] [US6] Add catalog test that activating a project revision does not insert one module per routine in `backend/crates/kalamdb-system/src/providers/catalog/catalog_tests.rs`

### Implementation

- [x] T025 [US6] Delete `module_id_for(routine_id)` usage from invoke/activation in `backend/crates/kalamdb-core/src/functions/acl.rs` and `backend/crates/kalamdb-core/src/functions/executor.rs`
- [x] T026 [US6] Resolve project module id from `[functions] module` / catalog `backend` in `backend/crates/kalamdb-core/src/functions/executor.rs` and `backend/crates/kalamdb-functions/src/activation.rs`
- [x] T027 [US6] Keep inline CREATE from inserting `CatalogFunctionModule` rows in `backend/crates/kalamdb-handlers/crates/ddl/src/procedure/create.rs`

**Checkpoint**: Hundreds of procedures do not create hundreds of modules.

---

## Phase 6: User Story 5 - Project Files Override Inline Fallbacks (Priority: P1)

**Goal**: Activation resolves module export, else inline, else missing; rollback without override restores inline with no DDL.

**Independent Test**: Inline `"basic"` → deploy file `"project"` → rollback → `"basic"`.

### Tests

- [x] T028 [P] [US5] Add activation/resolution tests (module beats inline; missing export restores inline) in `backend/crates/kalamdb-core/src/functions/` tests or `backend/crates/kalamdb-functions/src/activation.rs` tests

### Implementation

- [x] T029 [US5] Build `ActiveFunctionSet.procedures` from manifest + catalog inline state at CAS publish in `backend/crates/kalamdb-core/src/functions/runtime_state.rs` and `backend/crates/kalamdb-functions/src/activation.rs`
- [x] T030 [US5] Lookup `ImplementationRef` only from the pinned set (no filesystem) in `backend/crates/kalamdb-core/src/functions/executor.rs`

**Checkpoint**: Override precedence is an activation property, not a per-CALL file check.

---

## Phase 7: User Story 3 - Generate Bindings Once, Never Overwrite (Priority: P1)

**Goal**: Scaffold `functions/src/<schema>/<procedure>.ts` once; regenerate registry/contracts; never overwrite developer files; dispatch via compiled registry.

**Independent Test**: Edit a scaffold, regenerate, file unchanged, registry updated.

### Tests

- [x] T031 [P] [US3] Extend `cli/src/workflow/schema/generate_tests.rs` to assert implementation files are not overwritten and registry maps qualified names

### Implementation

- [x] T032 [US3] Keep one-shot scaffold and registry generation in `cli/src/workflow/schema/typescript.rs`; registry remains the build entrypoint
- [x] T033 [US3] Ensure runtime dispatch uses compiled registry exports, not `src/` paths, in `backend/crates/kalamdb-functions/src/v8_adapter.rs`

**Checkpoint**: Codegen is schema-first and non-destructive.

---

## Phase 8: User Story 23 - One Host Context API for Inline and Project (Priority: P1)

**Goal**: Shared generated `runtime.d.ts` for `ctx` / `defineProcedure`; inline typecheck via shim; one bootstrap shape.

**Independent Test**: Inline and project typecheck against the same `.d.ts`; regenerate does not overwrite `src/**`.

### Tests

- [x] T034 [P] [US23] Add generate tests for `.kalam/generated/runtime.d.ts` and inline shims in `cli/src/workflow/schema/generate_tests.rs`

### Implementation

- [x] T035 [US23] Emit `functions/.kalam/generated/runtime.d.ts` from `cli/src/workflow/schema/typescript.rs` per `specs/034-schema-first-functions/contracts/host-api.md`
- [x] T036 [US23] Move `defineProcedure` typing to runtime.d.ts (contracts may re-export) in `cli/src/workflow/schema/typescript.rs`
- [x] T037 [US23] Emit inline typecheck shims under `functions/.kalam/generated/inline/` in `cli/src/workflow/schema/typescript.rs`
- [x] T038 [US23] Converge `HOST_BOOTSTRAP` / `ASYNC_HOST_BOOTSTRAP` to one ABI v2 `ctx` (`db.query`/`execute`, http request/response) in `backend/crates/kalamdb-functions/src/wrap.rs`

**Checkpoint**: One host surface for both implementation kinds.

---

## Phase 9: User Story 2 - Full TypeScript Project (Priority: P1)

**Goal**: Normal project (package.json, lockfile, helpers, pure JS deps); server never runs npm; unsupported packages fail at build.

**Independent Test**: Two procedures sharing a helper and a pure JS package build offline and both invoke.

### Tests

- [x] T039 [P] [US2] Add CLI tests that native `.node` / Node builtin packages are rejected in `cli/src/workflow/functions.rs` tests or `cli` workflow test module

### Implementation

- [x] T040 [US2] Validate package graph (pure JS/TS ok; Node builtin / N-API rejected) during build in `cli/src/workflow/functions.rs`
- [x] T041 [US2] Hash lockfile into the artifact manifest in `cli/src/workflow/functions.rs` (full bundle lands in US16)

**Checkpoint**: Project shape is real; server is not a package manager.

---

## Phase 10: User Story 9 - Actor Stays Fixed; Principal Follows Security (Priority: P1)

**Goal**: Immutable actor; frame principal INVOKER/DEFINER; EXECUTE before enter; principal session cache; DEFINER search-path hardening.

**Independent Test**: User → DEFINER → nested INVOKER; actor unchanged; principal restores; RLS uses effective principal.

### Tests

- [x] T042 [P] [US9] Add nested INVOKER/DEFINER actor/principal tests in `backend/crates/kalamdb-core/src/functions/` tests
- [x] T043 [P] [US9] Add EXECUTE-denied-before-enter tests in `backend/crates/kalamdb-core/src/functions/acl.rs` tests

### Implementation

- [x] T044 [US9] Introduce `FunctionExecutionRoot` with immutable actor in `backend/crates/kalamdb-core/src/functions/executor.rs` and `backend/crates/kalamdb-core/src/functions/call_types.rs`
- [x] T045 [US9] Change `ProcedureFrame` to `SmallVec<[ProcedureFrame; 4]>` (max depth 16) in `backend/crates/kalamdb-core/src/functions/call_types.rs` and `backend/crates/kalamdb-core/src/functions/host.rs`
- [x] T046 [US9] Cache derived `ExecutionContext` per principal on the root (`SmallVec<[(PrincipalKey, ExecutionContext); 2]>`) in `backend/crates/kalamdb-core/src/functions/host.rs`; stop calling `with_effective_identity` on every `current_exec_ctx()` SQL hop
- [x] T047 [US9] Resolve generated DB accessors by canonical `TableId` / schema-qualified names; DEFINER raw SQL uses definer namespace search_path only in `backend/crates/kalamdb-core/src/functions/host.rs`
- [x] T048 [US9] Ensure JS cannot set actor/principal in `backend/crates/kalamdb-functions/src/wrap.rs` and `backend/crates/kalamdb-functions/src/v8_adapter.rs`

**Checkpoint**: Identity and ACL match spec US9 without rebuilding DataFusion per query.

---

## Phase 11: User Story 15 - Unauthenticated HTTP Requires Explicit Grant (Priority: P1)

**Goal**: URL existence is not anonymous EXECUTE. `PUBLIC` does not include anonymous.

**Independent Test**: REST without credentials denied until `GRANT EXECUTE` to anonymous.

### Tests

- [x] T049 [P] [US15] Add ACL tests: anonymous denied by default; explicit `RoutineGrantee::Anonymous` allowed; PUBLIC does not match Anonymous in `backend/crates/kalamdb-core/src/functions/acl.rs`

### Implementation

- [x] T050 [US15] Change `require_execute` so Anonymous is denied unless an explicit anonymous grant exists in `backend/crates/kalamdb-core/src/functions/acl.rs`
- [x] T051 [US15] Parse `GRANT EXECUTE ... TO` anonymous in `backend/crates/kalamdb-handlers/crates/ddl/src/procedure/grant.rs`

**Checkpoint**: Secure default for HTTP procedures.

---

## Phase 12: User Story 8 - One Root, One Transaction, Nested Stay Inside (Priority: P1)

**Goal**: One root per origin; nested calls share transaction/lease/pinned set; no nested `FunctionEngine::invoke` admission.

**Independent Test**: A→B→C one transaction; nested succeeds when root concurrency is already at max.

### Tests

- [x] T052 [P] [US8] Add engine tests that nested calls do not take another root/active slot in `backend/crates/kalamdb-functions/src/engine.rs` tests
- [x] T053 [P] [US8] Add host tests that nested CALL rolls back with the root on failure in `backend/crates/kalamdb-core/src/functions/` tests

### Implementation

- [x] T054 [US8] Checkout `RuntimeLease` only for depth-0 in `backend/crates/kalamdb-functions/src/engine.rs`; remove `nested_reserve` from `backend/crates/kalamdb-functions/src/engine_config.rs`
- [x] T055 [US8] Refactor `CoreFunctionHost` nested `call` to push frame and invoke on the current isolate (no `FunctionEngine::invoke`) in `backend/crates/kalamdb-core/src/functions/host.rs` and `backend/crates/kalamdb-functions/src/v8_async.rs`
- [x] T056 [US8] Delete ABI v1 `spawn_blocking` path in `backend/crates/kalamdb-functions/src/engine.rs` after tests use ABI v2
- [x] T057 [US8] Share root transaction for nested procs, table triggers, and topic publish; fail → full rollback in `backend/crates/kalamdb-core/src/functions/executor.rs` and `backend/crates/kalamdb-core/src/functions/runtime_state.rs`
- [x] T058 [US8] Enforce max depth 16 on the frame stack in `backend/crates/kalamdb-core/src/functions/host.rs`

**Checkpoint**: Nested work is reentrant on one lease.

---

## Phase 13: User Story 7 - In-Flight Calls Survive Deploy (Priority: P1)

**Goal**: Root pins `Arc<ActiveFunctionSet>`; nested children cannot observe a newly activated revision.

**Independent Test**: Slow A on 41, activate 42, B uses 42, A including nested stays on 41.

### Tests

- [x] T059 [P] [US7] Add pin/race test (A on 41, B on 42) in `backend/crates/kalamdb-core/src/functions/` tests

### Implementation

- [x] T060 [US7] Clone `Arc<ActiveFunctionSet>` onto `FunctionExecutionRoot` at start in `backend/crates/kalamdb-core/src/functions/executor.rs`
- [x] T061 [US7] Nested lookup uses the root pin only in `backend/crates/kalamdb-core/src/functions/host.rs`
- [x] T062 [US7] After CAS, `ArcSwap::store` new set so only new roots see it in `backend/crates/kalamdb-core/src/functions/runtime_state.rs`

**Checkpoint**: Deploy/rollback cannot split a request across revisions.

---

## Phase 14: User Story 10 - Global Memory and Concurrency Budget (Priority: P1)

**Goal**: Conservative workers; global memory ceiling; revision-keyed small idle pool; bounded queue; nested does not multiply heap.

**Independent Test**: Modest `max_memory_mb`; admission/backpressure; RSS tracks budget not `max_active × max_heap`.

### Tests

- [x] T063 [P] [US10] Add admission tests (capacity, queue bound, memory budget) in `backend/crates/kalamdb-functions/src/engine.rs` tests

### Implementation

- [x] T064 [US10] Default `workers = min(cpus, 4)` and drop half-CPU default in `backend/crates/kalamdb-functions/src/engine_config.rs`
- [x] T065 [US10] Implement global memory accounting and dual admission (active + memory) in `backend/crates/kalamdb-functions/src/engine.rs`
- [x] T066 [US10] Create lanes lazily; do not spawn `max_active` isolates at engine start in `backend/crates/kalamdb-functions/src/engine.rs`
- [x] T067 [US10] Key idle pool by `FunctionRevisionId`; max 0–1 idle per lane/revision; no cross-revision `rebind()` in `backend/crates/kalamdb-functions/src/engine.rs` and `backend/crates/kalamdb-functions/src/v8_adapter.rs`
- [x] T068 [US10] Soft heap recycle vs destroy; drop idle on superseded revisions in `backend/crates/kalamdb-functions/src/v8_adapter.rs`
- [x] T069 [US10] Queue lightweight Arc-backed roots only; reject oversized inputs before enqueue using `max_value_bytes` in `backend/crates/kalamdb-functions/src/engine.rs`

**Checkpoint**: Function memory is an operator-configured ceiling.

---

## Phase 15: User Story 11 - Pooled Executions Cannot Leak State (Priority: P1)

**Goal**: Fresh request context; destroy isolate if cleanup unproven; timeouts cancel JS, host, DB, nested, queue.

**Independent Test**: Module-global actor leak returns null/undefined on the next request; timeout then B sees no A state.

### Tests

- [x] T070 [P] [US11] Add pool leak test (module `let leaked`) in `backend/crates/kalamdb-functions/src/v8_adapter.rs` tests
- [x] T071 [P] [US11] Add timeout-then-reuse isolation test in `backend/crates/kalamdb-functions/src/engine.rs` tests

### Implementation

- [x] T072 [US11] Recreate request context / drop host pointers / drain promises before idle return in `backend/crates/kalamdb-functions/src/v8_adapter.rs` and `backend/crates/kalamdb-functions/src/v8_async.rs`
- [x] T073 [US11] Destroy isolate when cleanup, heap-soft, or cancellation cannot be proven in `backend/crates/kalamdb-functions/src/engine.rs`
- [x] T074 [US11] Timeout/cancel covers JS, host futures, SQL, nested, and queued roots in `backend/crates/kalamdb-functions/src/deadline.rs` and `backend/crates/kalamdb-core/src/functions/executor.rs`

**Checkpoint**: Pooling does not weaken isolation.

---

## Phase 16: User Story 14 - Small Explicit Capability Set (Priority: P1)

**Goal**: No fs/net/process/eval/Node; thin native host; host-op limits; lazy DataFusion; no JSON nested ABI.

**Independent Test**: eval/fs/net/native package fail; nested typed args; `return a+b` does not create a SessionContext.

### Tests

- [x] T075 [P] [US14] Add sandbox tests (eval, filesystem, network) in `backend/crates/kalamdb-functions/src/v8_adapter.rs` tests
- [x] T076 [P] [US14] Add host-op limit tests (too many calls, huge return, huge DB result) in `backend/crates/kalamdb-functions/src/limits.rs` tests

### Implementation

- [x] T077 [US14] Disable eval/`Function`, omit Node builtins, and fail deploy if bundle needs eval in `backend/crates/kalamdb-functions/src/v8_adapter.rs` and `cli/src/workflow/functions.rs`
- [x] T078 [US14] Enforce host-op limits (SQL length, result rows/bytes, nested depth, topics, logs, headers, return/input bytes) in `backend/crates/kalamdb-functions/src/limits.rs` and `backend/crates/kalamdb-core/src/functions/host.rs`
- [x] T079 [US14] Keep DataFusion lazy: no `SessionContext` until first host DB op in `backend/crates/kalamdb-core/src/functions/host.rs`

**Checkpoint**: Capabilities are only `ctx.*`.

---

## Phase 17: User Story 24 - FlatBuffer Zero-Copy JS Boundary (Priority: P1)

**Goal**: Request/response crossing V8 reuses a contract-matching FlatBuffer or encodes once; nested calls share `Bytes`; no JSON at the isolate hop.

**Independent Test**: Already-FB input not re-encoded; non-FB encodes once; nested reuses buffer; REST JSON only at HTTP edge.

### Tests

- [x] T080 [P] [US24] Add transfer-codec tests (reuse vs one encode, contract-hash mismatch re-encodes) in `backend/crates/kalamdb-serialization/src/function_value.rs`
- [x] T081 [P] [US24] Add nested-call buffer-reuse test in `backend/crates/kalamdb-functions/src/convert.rs` tests

### Implementation

- [x] T082 [US24] Add in-memory function FlatBuffer codec in `backend/crates/kalamdb-serialization/src/function_value.rs` and export from `backend/crates/kalamdb-serialization/src/lib.rs`
- [x] T083 [US24] Replace JSON/field-graph copies at the V8 hop with zero-copy `ArrayBuffer` or one encode in `backend/crates/kalamdb-functions/src/convert.rs`
- [x] T084 [US24] Pass `bytes::Bytes` for nested in-process calls (not `encode_object`, not JSON) in `backend/crates/kalamdb-core/src/functions/host.rs`
- [x] T085 [US24] Convert HTTP JSON once at the REST edge then enter the FB path in `backend/crates/kalamdb-api/src/http/functions.rs`

**Checkpoint**: Isolate serdes is one buffer, not a JSON roundtrip.

---

## Phase 18: User Story 12 - HTTP Context Compact and Safe (Priority: P2)

**Goal**: On-demand headers; block credentials; nested cannot mutate response; reject hop-by-hop response headers.

**Independent Test**: `x-client-version` readable; `Authorization` not; nested cannot set status; `Connection` rejected.

### Tests

- [x] T086 [P] [US12] Add HTTP context unit tests in `backend/crates/kalamdb-core/src/functions/call_types.rs` tests and `backend/crates/kalamdb-api/src/http/functions.rs` tests

### Implementation

- [x] T087 [US12] Store `Arc<HeaderMap>` (or equivalent) on `HttpInvocationContext` instead of `HashMap<String,String>` in `backend/crates/kalamdb-core/src/functions/call_types.rs` and `backend/crates/kalamdb-api/src/http/functions.rs`
- [x] T088 [US12] Expose method/path/`headers.get`/`query.get`; block Authorization, Proxy-Authorization, raw Cookie in `backend/crates/kalamdb-functions/src/wrap.rs` and host HTTP callbacks in `backend/crates/kalamdb-core/src/functions/host.rs`
- [x] T089 [US12] Root-only response mutation; reject Connection/Transfer-Encoding/Content-Length/Host; enforce header size limits in `backend/crates/kalamdb-core/src/functions/host.rs`
- [x] T090 [US12] Set `ctx.http` null for SQL/trigger/schedule origins in `backend/crates/kalamdb-core/src/functions/executor.rs`

**Checkpoint**: HTTP metadata is least-privilege and allocation-light.

---

## Phase 19: User Story 13 - REST Body Is the SQL Result (Priority: P2)

**Goal**: Success body = declared return type; errors use typed codes → HTTP status.

**Independent Test**: `{order_id}` not wrapped; 404/403/401/400/429/504/500 from codes, not `contains("denied")`.

### Tests

- [x] T091 [P] [US13] Add REST mapping tests in `backend/crates/kalamdb-api/src/http/functions.rs` tests

### Implementation

- [x] T092 [US13] Return SQL JSON as the HTTP body with no `{status,result}` wrapper in `backend/crates/kalamdb-api/src/http/functions.rs`
- [x] T093 [US13] Map error **code fields** to HTTP status per `specs/034-schema-first-functions/contracts/typed-errors.md` in `backend/crates/kalamdb-api/src/http/functions.rs`; delete message-substring matching

**Checkpoint**: REST contract matches the SQL procedure type.

---

## Phase 20: User Story 16 - `functions build` Is a Real Build (Priority: P2)

**Goal**: Offline contract compile, generate, typecheck, bundle, manifest, export validation.

**Independent Test**: Missing export and unknown export fail; valid project writes artifact + manifest without a server.

### Tests

- [x] T094 [P] [US16] Add CLI tests for missing/unknown exports and offline success in `cli/src/workflow/` tests

### Implementation

- [x] T095 [US16] Replace generate-only `build_functions` with the full pipeline in `cli/src/workflow/functions.rs`
- [x] T096 [US16] Write `functions/.kalam/build/manifest.json` per `specs/034-schema-first-functions/contracts/build-manifest.md` in `cli/src/workflow/functions.rs`
- [x] T097 [US16] Validate registry exports vs ContractSnapshot (module vs inline) in `cli/src/workflow/functions.rs`

**Checkpoint**: Build is honest and offline.

---

## Phase 21: User Story 17 - Deploy CAS + Dry-Run Validates Locally (Priority: P2)

**Goal**: Deploy uploads immutable artifact, applies catalog, CAS active pointer; dry-run does all local work with zero mutations.

**Independent Test**: `--dry-run` builds and prints plan with no catalog change; real deploy CAS-activates and leaves old artifacts immutable.

### Tests

- [x] T098 [P] [US17] Extend `cli/src/workflow/deploy/mod.rs` dry-run test to assert generate/build ran and no upload/migrate
- [x] T099 [P] [US17] Add activation mismatch tests (contract/ABI/export) in `backend/crates/kalamdb-functions/src/activation.rs` tests

### Implementation

- [x] T100 [US17] Sequence deploy: snapshot → generate → schema diff → functions build → upload → catalog revision → CAS in `cli/src/workflow/deploy/mod.rs`
- [x] T101 [US17] Refuse activation on hash/ABI/export mismatch; keep previous active set in `backend/crates/kalamdb-functions/src/activation.rs`
- [x] T102 [US17] Load artifact bytes once into `Arc<ModuleRevision>`; no per-CALL filestore read in `backend/crates/kalamdb-functions/src/engine.rs` and `backend/crates/kalamdb-core/src/functions/executor.rs`
- [x] T103 [US17] Make `--dry-run` run parse/diff/generate/typecheck/build/hash/plan only in `cli/src/workflow/deploy/mod.rs`

**Checkpoint**: Schema and artifact cannot diverge; dry-run is useful.

---

## Phase 22: User Story 18 - Revisions, Status, Rollback, Logs (Priority: P2)

**Goal**: Operator CLI lists revisions and rolls back by pointer swap.

**Independent Test**: Deploy twice, list active/ready, rollback, old behavior returns.

### Tests

- [x] T104 [P] [US18] Add rollback compatibility tests (missing artifact, bad ABI, contract mismatch) in `backend/crates/kalamdb-functions/src/activation.rs` tests

### Implementation

- [x] T105 [US18] Implement `kalam functions revisions` and `status` in `cli/src/workflow/functions.rs`
- [x] T106 [US18] Implement rollback as CAS pointer swap (no rebuild) in `cli/src/workflow/functions.rs` and `backend/crates/kalamdb-functions/src/activation.rs`
- [x] T107 [US18] Implement `kalam functions logs` from structured function errors (not only trigger_attempts) in `cli/src/workflow/functions.rs`

**Checkpoint**: Rollback is a product command, not an error stub.

---

## Phase 23: User Story 20 - Inline Not Auto-Scaffolded (Priority: P2)

**Goal**: Generate/dev do not create override files for inline procs; `kalam functions override` does.

**Independent Test**: Inline health has no `src/api/health.ts` until override.

### Tests

- [x] T108 [P] [US20] Add generate tests that inline routines skip scaffold in `cli/src/workflow/schema/generate_tests.rs`

### Implementation

- [x] T109 [US20] Skip auto-scaffold when catalog/snapshot marks inline in `cli/src/workflow/schema/typescript.rs`
- [x] T110 [US20] Implement `kalam functions override <schema.proc>` in `cli/src/workflow/functions.rs`, optionally seeding from inline body

**Checkpoint**: Inline remains fallback until the developer opts in.

---

## Phase 24: User Story 19 - `kalam dev` Without Database Restart (Priority: P2)

**Goal**: Watch schema + functions; regenerate/scaffold/typecheck on SQL; incremental bundle + local activate on TS; no server restart.

**Independent Test**: Add procedure SQL, scaffold appears, implement, save, CALL new behavior without restart.

### Implementation

- [x] T111 [US19] Watch `schema/**/*.sql`, `functions/src/**`, `package.json`, lockfile, `tsconfig.json` in the existing CLI dev workflow (`cli/src/workflow/` dev entry — extend the current `kalam dev` watcher)
- [x] T112 [US19] On schema change: ContractSnapshot + generate + scaffold missing non-inline + typecheck in that watcher
- [x] T113 [US19] On function change: incremental bundle, local revision, CAS/activate without restarting the server process

**Checkpoint**: Edit-save-invoke loop does not bounce KalamDB.

---

## Phase 25: User Story 21 - Observability Without Secrets (Priority: P3)

**Goal**: Low-cardinality metrics; structured errors; in-memory `system.active_function_runs`.

**Independent Test**: Invoke/fail/timeout; scrape metrics without user_id labels; active run appears then vanishes; logs omit Authorization/bodies.

### Tests

- [x] T114 [P] [US21] Add metrics/label tests in `backend/crates/kalamdb-observability/src/function_metrics.rs`
- [x] T115 [P] [US21] Add active-runs virtual table tests in `backend/crates/kalamdb-system` or `backend/crates/kalamdb-core` function runtime tests

### Implementation

- [x] T116 [US21] Extend counters/histograms (invocations, errors, timeouts, oom, duration, queue wait, instance create/destroy, heap, revision cache, host DB, nested depth) in `backend/crates/kalamdb-observability/src/function_metrics.rs` with low-cardinality labels only
- [x] T117 [US21] Structured error logs (execution_id, request_id, routine, revision, actor, principal, source, stack, code) without credentials/bodies in `backend/crates/kalamdb-core/src/functions/executor.rs`
- [x] T118 [US21] Track active roots in memory and expose `system.active_function_runs` in `backend/crates/kalamdb-core/src/functions/runtime_state.rs` plus a system table provider under `backend/crates/kalamdb-system/src/`

**Checkpoint**: Operators can debug without a per-CALL RocksDB row or secret logs.

---

## Phase 26: User Story 22 - Init-to-Rollback E2E (Priority: P2)

**Goal**: Full developer journey against a running server.

**Independent Test**: `kalam init` → SQL → `kalam dev` → implement with helper + pure npm dep → build → REST + CALL → deploy → revisions → change → deploy → rollback.

### Tests

- [x] T119 [US22] Add CLI e2e covering the US22 journey (server required) under `cli` e2e tests, following `specs/034-schema-first-functions/quickstart.md`

**Checkpoint**: The product loop works end to end.

---

## Phase 27: Polish & Cross-Cutting Concerns

**Purpose**: Docs, optional snapshot, Stage 8 gates.

- [x] T120 [P] Amend `docs/architecture/decisions/adr-021-kalamdb-functions.md` (one project module, ABI 2, Wasmtime not default, FlatBuffer JS transfer vs durable Arrow/SQL model)
- [x] T121 [P] Update `docs/reference/sql.md` for LANGUAGE-only-with-body, inline vs project, REST body, GRANT anonymous
- [x] T122 [P] Update CLI help for build/status/revisions/rollback/override/dry-run in `cli/src/workflow/functions.rs` and deploy help in `cli/src/workflow/deploy/mod.rs`
- [x] T123 Update `../kalamdb-skills` canonical SQL/CLI content if that tree is reachable; otherwise record skip in `specs/034-schema-first-functions/validation/skills-docs.md`
- [x] T124 After Stage 0 vs Stage 3 numbers, optionally add static V8 startup snapshot in `backend/crates/kalamdb-functions/build.rs` and load via `CreateParams::snapshot_blob` in `backend/crates/kalamdb-functions/src/v8_adapter.rs` (skipped: init cost was not isolated in Stage 0; no user-revision snapshots)
- [x] T125 Re-run Stage 0 benches/RSS/binary size into `specs/034-schema-first-functions/validation/stage8-gate.md`; confirm Wasmtime not linked in default `kalamdb-server`; confirm SC-018/SC-019; run `specs/034-schema-first-functions/quickstart.md` Stage 8 commands

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: Start immediately (T001 is the hard gate before refactors)
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories
- **US1 → US4 → US6 → US5**: Schema-first catalog/module/resolution (plan Stage 1)
- **US3 → US23 → US2**: Codegen and project (can overlap CLI after US1)
- **US9 → US15**: Identity then anonymous HTTP
- **US8 → US7 → US10 → US11**: Runtime lease, pin, memory, pool (plan Stage 3)
- **US14**: After US8 host exists; can overlap US10
- **US24**: After US8 nested path exists; REST edge with US13
- **US12 → US13**: HTTP then REST body
- **US16 → US17 → US18 → US20 → US19 → US22**: CLI/deploy (plan Stages 4–6)
- **US21**: After roots exist (US8)
- **Polish**: After desired stories; T124 only after measurement; T125 last

### User Story Dependencies

| Story | Depends on |
|-------|------------|
| US1 | Foundational |
| US4 | US1 |
| US6 | US1 |
| US5 | US4, US6 |
| US3 | US1 |
| US23 | US3 |
| US2 | US3, US23 |
| US9 | Foundational (uses ActiveFunctionSet) |
| US15 | US9 ACL |
| US8 | US9 frames |
| US7 | US5, US8 |
| US10 | US8 |
| US11 | US10 |
| US14 | US8 |
| US24 | US8 |
| US12 | US8 root HTTP |
| US13 | US12, T006 codes |
| US16 | US2, US3, US23 |
| US17 | US16, US6, US7 |
| US18 | US17 |
| US20 | US3, US4 |
| US19 | US16, US17 |
| US21 | US8 |
| US22 | US16–US20 |

### Parallel Opportunities

- T004/T005 after T002
- T015/T016; T031/T034 generate tests
- US3 codegen vs US1 parser (different crates) after Foundational
- US12 HTTP vs US16 CLI after US8
- T120/T121/T122 docs in parallel during polish

### Parallel Example: User Story 9

```bash
Task: "Add nested INVOKER/DEFINER actor/principal tests in backend/crates/kalamdb-core/src/functions/"
Task: "Add EXECUTE-denied-before-enter tests in backend/crates/kalamdb-core/src/functions/acl.rs"
```

---

## Implementation Strategy

### MVP First (Schema-first CALL)

1. Phase 1 Setup including **T001 baseline**
2. Phase 2 Foundational
3. US1 + US4 + US6 + US5 (callable contracts, inline JS, one module, resolution)
4. **STOP and VALIDATE** Independent Tests for those stories

### Incremental Delivery

1. Codegen/host types: US3, US23, US2
2. Security: US9, US15, US14
3. Runtime: US8, US7, US10, US11, US24
4. HTTP/REST: US12, US13
5. CLI: US16, US17, US18, US20, US19
6. Observe + e2e: US21, US22
7. Polish + Stage 8 gate

### Parallel Team Strategy

After Foundational: dialect/catalog (US1/US4/US6) ∥ CLI generate (US3/US23) ∥ core identity (US9). Runtime lease (US8+) should not start until US5 resolution types exist.

---

## Notes

- Do not preserve `module_id_for(routine_id)`, nested engine admission, REST success wrapper, substring HTTP status, generate-only build, rollback stub, or default Wasmtime if replacement tests pass
- Preserve CAS activation, ContractSnapshot, filestore artifacts, process-global V8 init, EXECUTE ACL, transaction coordinator
- Record bench times in seconds
- `tasks.md` is the implementation queue for `/speckit-implement`; do not mark complete on unit tests alone (T125)
