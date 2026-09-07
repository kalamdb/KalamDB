# Research: Schema-First Functions Runtime (V1.1)

All Technical Context unknowns from the spec are resolved below. No NEEDS CLARIFICATION remains.

## Default runtime: V8 only

**Decision**: `kalamdb-functions` default features are `["catalog"]` only. Wasmtime is not a workspace dependency. There is no `wasm-runtime` feature, WIT adapter, or `FunctionRuntime::Wasm`.

**Rationale**: ADR-021 executes TypeScript in V8. Keeping an unused Wasmtime adapter still compiled optional code and catalog variants that nothing deployed. Spec SC-020 required binary-size reduction; removing the crate is the complete form of that decision.

**Alternatives considered**: Keep default-on for “future WASM” — rejected (unused weight). Delete WASM code entirely — rejected (optional feature still useful for Stage 8 experimental benches, not the standard binary).

## Worker and admission defaults

**Decision**: `workers = min(available_parallelism, 4)` (configurable). `max_active` starts at 16 (configurable). Drop `nested_reserve` as the common nested strategy; nested calls do not take a root or extra active slot. Add `max_memory_mb` (start 256), `heap_soft_mb` (start 16), `heap_hard_mb` (start 64), `max_idle_per_lane` (0 or 1), `idle_ttl`, `max_queued` (128). Retune only after Stage 0 numbers.

**Rationale**: Current `workers = cpus/2`, `max_active = 32`, `nested_reserve = 8`, `max_heap = 64MiB`, `cache = 128MiB` allow a multi-gigabyte theoretical envelope. Spec US10.

**Alternatives considered**: Keep 32/64MiB and only document the product — rejected. Eager-create `max_active` isolates — rejected (idle cost).

## Engine laziness

**Decision**: Keep `FunctionRuntimeState` `OnceLock` so server start allocates no engine. Change `FunctionEngine::new` so lanes/isolates are created on demand, not `cpus/2` threads at first `engine()`. Isolates stay empty until a root needs one.

**Rationale**: Current engine is lazy at process start but eagerly spawns all worker threads on first invoke.

**Alternatives considered**: Pre-warm one isolate at server start — rejected (spec: idle unused functions ≈ no instance RSS).

## ActiveFunctionSet publish

**Decision**: Add `arc-swap` (workspace). Core holds `Arc<ArcSwap<ActiveFunctionSet>>`. Every root clones one `Arc<ActiveFunctionSet>`. Nested calls use that pin. CAS activation (existing Meta-Raft / `system.function_modules.active_revision_id`) rebuilds and swaps the set.

**Rationale**: Spec US7 — in-flight roots must not observe a mid-request deploy.

**Alternatives considered**: Re-read catalog per nested call — rejected. `RwLock` — extra contention. Persistent immutable map crate — overkill.

## One module, not `module_id_for(routine_id)`

**Decision**: Project module id comes from `[functions] module` (default `backend`). Delete `acl::module_id_for` as the project mapping. Inline routines are catalog metadata + optional compiled inline artifact, never `FunctionModuleId == RoutineId`.

**Rationale**: Current executor looks up `active_module(&module_id_for(&routine.routine_id))`, which creates one module per procedure.

**Alternatives considered**: Keep per-routine modules with a grouping table — rejected (hundreds of modules, caches, activations).

## Inline storage

**Decision**: Extend `CatalogRoutine` with `inline_source_hash: Option<String>` and `inline_artifact_id: Option<ArtifactId>`. Keep existing `language` and `body`. Compiled inline JS is content-addressed in filestore like project artifacts. Inline TypeScript: persist source; do not execute until a project build produced JS (spec assumption).

**Rationale**: Avoid a FunctionModule per inline routine. Hash avoids duplicating source as the execution key.

**Alternatives considered**: Separate `system.inline_routines` table — extra catalog surface for V1. Server-side tsc — rejected (binary/memory).

## Implementation resolution

**Decision**: At activation, for each `RoutineId`: if active revision manifest/registry exports it → `Module { revision, slot }`; else if inline compiled JS exists → `Inline { artifact }`; else `Missing`. Project file without being in the **active** revision does not override.

**Rationale**: Spec US5. Filesystem is never consulted per CALL.

**Alternatives considered**: Per-call “does src/file exist” — rejected.

## Root execution and nested reentry

**Decision**: `FunctionService::invoke` creates `FunctionExecutionRoot` once (REST, SQL CALL, wire, trigger, schedule). Checkout one `RuntimeLease` (lane + isolate). `CoreFunctionHost` nested `call` must **not** call `FunctionEngine::invoke`. It: EXECUTE ACL → push `ProcedureFrame` → invoke registered function on the current isolate → pop frame. Same transaction, deadline, cancellation, pinned set.

**Rationale**: Current nested path re-enters `invoke()`, round-robins another worker, and consumes `active` semaphore slots (`nested_reserve` workaround).

**Alternatives considered**: HTTP/SQL roundtrip for nested CALL — rejected. Shared isolate with a second engine admission — still wrong.

## Actor vs principal and ExecutionContext cache

**Decision**: Root stores immutable actor. Frames store effective principal. `PrincipalSessionCache` as `SmallVec<[(PrincipalKey, ExecutionContext); 2]>` on the root. DEFINER creates one derived context via existing `with_effective_identity` and reuses it. `ExecutionContext::Clone` continues to share cached DataFusion `SessionContext`. Pure procedures never touch the cache.

**Rationale**: `current_exec_ctx()` currently builds a fresh identity (and thus a new `OnceCell`) on every host SQL call.

**Alternatives considered**: Rewrite DataFusion session system — rejected by spec. Thread-local DF context — identity bugs.

## Search path / DEFINER

**Decision**: Generated `ctx.db.<schema>.<table>` resolves by canonical `TableId` / schema-qualified names. Raw `ctx.db.query` SQL inside DEFINER uses a deterministic search_path of the definer’s namespace only (not the caller’s writable schemas). Exact SET search_path API is not exposed to JS.

**Rationale**: Spec FR-028. Canonical IDs for generated access; safe path for raw SQL.

**Alternatives considered**: Ban raw SQL in DEFINER — too harsh for V1. Inherit caller search_path — classic shadowing attack.

## Anonymous EXECUTE

**Decision**: Keep default deny for `Role::Anonymous`. `GRANT EXECUTE ... TO` an explicit anonymous grantee (new `RoutineGrantee::Anonymous` or dedicated role mapping) is required for unauthenticated REST. **`PUBLIC` does not include anonymous.**

**Rationale**: Current code denies anonymous before grants, which is the right default, but `PUBLIC => true` would accidentally open HTTP if that early return were removed. Spec US15.

**Alternatives considered**: Treat PUBLIC as PostgreSQL PUBLIC (everyone) — rejected for this product’s HTTP surface.

## Shared host `.d.ts`

**Decision**: Generate `.kalam/generated/runtime.d.ts` (host `ctx`, `defineProcedure`) and keep SQL types in `.kalam/generated/contracts.ts`. Project `tsconfig` references both. `defineProcedure` moves out of a duplicated stub in contracts toward runtime.d.ts (contracts may re-export). Inline bodies typecheck via a generated shim file under `.kalam/generated/inline/` that wraps `export async function kalamInline(ctx: ProcedureContext, input: ...)` — not by importing from dollar-quoted SQL. Runtime bootstrap in `wrap.rs` implements **one** ctx shape matching the `.d.ts` (today ABI v1 `ctx.db.sql` vs v2 `ctx.db.query` must converge).

**Rationale**: Spec US23. Adding a host method = `.d.ts` + native host once.

**Alternatives considered**: Publish `@kalamdb/functions` npm package from the server repo for types — extra packaging for V1; generated files are enough. Dual ctx forever — rejected.

## JS boundary FlatBuffers

**Decision**: Add `kalamdb-serialization` in-memory function-value codec (contract-hash tagged FlatBuffer). Host↔V8: if `bytes` already match this contract hash and are FB, wrap as `v8::ArrayBuffer` backing store (zero-copy) or pass the same `bytes::Bytes` into nested calls. Else encode once from `RoutineValue`/`ScalarValue` or from parsed HTTP JSON. REST JSON happens at the HTTP edge only. Nested calls do not JSON and do not use `encode_object` (durable RocksDB codec). Mutated/new JS objects encode once on the way out. Unsafe lifetime/alignment → one owned FB copy, never JSON.

**Rationale**: Spec US24. ADR-021 “Arrow not bytes” meant “don’t persist-encode nested calls”. Arrow/`RoutineValue` remains the SQL typed model; FB is packing for the isolate hop. Central crate keeps codec ownership.

**Alternatives considered**: Keep field-by-field `convert.rs` only — still copies. JSON.stringify as the ABI — rejected. FlexBuffers only — FB matches existing workspace `flatbuffers` and zero-copy random access. Put codec in `kalamdb-functions` — violates ADR-021 / constitution ownership.

## ABI version

**Decision**: Production `ABI_VERSION = 2` (async host ops). Remove the `abi_version == 1` `spawn_blocking` path after migrating tests/fixtures. Manifest records `abiVersion: 2`. Activation rejects unsupported ABI.

**Rationale**: Spec FR-060/FR-062. Two architectures indefinitely is the current defect.

**Alternatives considered**: Keep v1 for “sync CPU procedures” — rejected (one ABI).

## HTTP REST

**Decision**: Keep `Arc<HeaderMap>` (or `Arc<HttpInvocationContext>` holding `HeaderMap`) on the root. Convert a header to JS string only on `headers.get`. Block `Authorization` / `Proxy-Authorization` by default. Cookie raw header blocked; no cookie API in V1 unless added later. Success body = SQL return JSON (no `{status,result}`). Errors = existing Kalam error envelope + typed codes (see contracts/typed-errors.md). Nested frames cannot mutate response.

**Rationale**: Spec US12–US13. Current handler stringifies all headers and uses `message.contains`.

**Alternatives considered**: Keep success wrapper for compatibility — rejected (spec explicit).

## Pooling and snapshots

**Decision**: Idle pool keyed by `FunctionRevisionId`, ≤1 idle per lane/revision by default. Do not `rebind()` an isolate onto an unrelated revision. Cleanup checklist from spec US11; destroy if unproven. Stage 7: static bootstrap snapshot via existing v8 152 `SnapshotCreator` + `CreateParams::snapshot_blob`; no V8 bump; no user-code snapshot.

**Rationale**: Current `run_v8` pops any idle session and `rebind()`s. Spec US24/§29.

**Alternatives considered**: Snapshot every user revision — deferred until measured.

## CLI build / deploy / rollback

**Decision**: `kalam functions build` = contract compile + generate + typecheck/bundle (esbuild/project script; **no live server**) + manifest + export validation. Deploy uploads artifact, migrates catalog, CAS active pointer, optional one warm instance. `--dry-run` runs all local steps, no upload/migrate/CAS. Rollback = CAS pointer to an existing revision after artifact/ABI/contract checks. `kalam functions override name` scaffolds from inline. `kalam dev` watches schema + functions without DB restart.

**Rationale**: Current build is `generate_schema` only; rollback returns an error; dry-run skips generate.

**Alternatives considered**: Require a running server to build — rejected.

## Metrics and active runs

**Decision**: Extend `kalamdb-observability::function_metrics` with spec US21 counters/histograms (low cardinality). Keep in-memory active runs; expose `system.active_function_runs` as a virtual provider (no RocksDB row per invoke). Current `begin_function_run` / `finish_function_run` is the seed.

**Rationale**: Spec US21. Don’t invent a second metrics crate.

**Alternatives considered**: Prometheus labels with `routine_id` high cardinality — only allow a bounded set of routine names if already catalog-sized; prefer no user_id/request_id. If routine_id cardinality is a concern, omit it from Prometheus and keep it in structured logs only. **Decision**: metrics labels = `result`/`error_code` class only; `routine_id` stays in logs.

## Experimental runtimes

**Decision**: Do not ship QuickJS, Wasmtime, or native-Rust function runtimes. Wasmtime is not a workspace dependency.

**Rationale**: Spec §28 / FR-081.
