# Implementation Plan: Schema-First Functions Runtime (V1.1)

**Branch**: `034-schema-first-functions` | **Date**: 2026-09-07 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/034-schema-first-functions/spec.md`

**Note**: This plan evolves the existing Functions implementation (PR #380 / `feat/server-functions`). It does not design Functions from scratch.

## Summary

Evolve the current Functions stack into a **schema-first, one-module-per-project** runtime: SQL remains the contract; a TypeScript project (or inline JS fallback) is the implementation; KalamDB executes, deploys, and rolls back. Fix the current architectural defects before adding features: `module_id = routine_id`, nested calls re-entering engine admission, `max_active=32 × 64MiB` unbounded envelope, per-query `ExecutionContext` rebuild, Wasmtime in default features, REST `{status,result}` wrapper, string-matched HTTP errors, `functions build` as generate-only, and unimplemented rollback.

Technical approach: publish an immutable **`ActiveFunctionSet`** (atomic pointer swap); give every origin one **`FunctionExecutionRoot`** with a **root `RuntimeLease`**; nested `ctx.functions.*` stays on that lease; share one generated **`runtime.d.ts`** for inline and project `ctx`; cross the JS boundary with a **FlatBuffer zero-copy check** owned by `kalamdb-serialization`; converge on **ABI v2 async** only; keep V8 and do not ship Wasmtime; measure before startup snapshots.

## Technical Context

**Language/Version**: Rust 1.94 (workspace edition 2021); TypeScript/JavaScript for function projects (compiled off-server); SQL dialect for contracts

**Primary Dependencies**: Existing — `tokio`, `v8` 152.2.0, `datafusion` 55.x, `arrow`, `moka`, `dashmap`, `bytes`, `flatbuffers` 25.12.19, `kalamdb-serialization`, `kalamdb-filestore`, `kalamdb-system`, `kalamdb-core`, `kalamdb-api`, `kalamdb-dialect`, `kalamdb-handlers`, `kalamdb-observability`, `kalamdb-configs`, CLI workflow. New (workspace-pinned, smallest feature set) — `arc-swap` (active-set publish), `smallvec` (procedure frames / principal cache). **Wasmtime is not a dependency.** **Do not** add Deno, QuickJS, TypeScript compiler, or npm into the server.

**Storage**: Catalog in RocksDB via `kalamdb-system` (`system.routines`, `system.function_modules`, `system.function_revisions`, artifacts). Function artifacts in `kalamdb-filestore` (content-addressed). Active set and active runs are **in-memory only**. Durable row/object codecs stay in `kalamdb-serialization` — function FlatBuffers are an **in-memory transfer format**, not a second on-disk row codec.

**Testing**: `cargo nextest run` on `kalamdb-functions`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-handlers`, `kalamdb-api`, `kalamdb-system`, CLI e2e; `benchv2/comparison/drivers/kalamdb_functions`; RSS/binary-size capture in Stage 0 and Stage 8. Record benchmark runtimes in **seconds**.

**Target Platform**: Linux/macOS KalamDB server (aarch64, x86_64); CLI on developer machines / CI

**Project Type**: Multi-crate Rust database engine + CLI

**Performance Goals**: After Stage 0 baseline: no-op warm invocation regression ≤10% (SC-018); DB-heavy throughput regression ≤5%; nested call does not allocate a second isolate (SC-019); idle server with unused functions adds no instance RSS (SC-008); functions memory stays inside configured `max_memory_mb` (SC-009)

**Constraints**: One database process — no extra services/IPC. V1 is DBA-deployed trusted code, not a hostile multi-tenant sandbox. Nested calls share the root transaction. `kalamdb-core` must not depend on `V8Session`. Do not switch production runtime. Do not implement user-revision snapshots in this change. Do not run `npm install` or a TypeScript compiler inside the server.

**Scale/Scope**: One functions module (`backend`) with many procedures; default ≤4 runtime lanes; global memory ceiling (start 256MiB, retune after baseline); max nesting 16; catalog + CLI + REST + SQL CALL + triggers. Touches the files listed in spec §80 plus serialization, configs, observability, and docs/ADR-021.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Assessment | Gate |
|-----------|------------|------|
| **I. Performance-First** | Stage 0 captures baseline before refactors. Nested calls stop allocating extra isolates. Global memory budget replaces `32×64MiB`. DataFusion stays lazy. FlatBuffer reuse avoids JSON at the JS boundary. Startup snapshot is Stage 7 **after** measurement. Wasmtime is not in the workspace. | PASS |
| **II. Boundary Ownership** | V8/ABI/pool → `kalamdb-functions`. Host SQL/ACL/tx → `kalamdb-core`. Catalog → `kalamdb-system`. Artifacts → `kalamdb-filestore`. Transfer codec → `kalamdb-serialization` (not a functions-owned persistence fork). HTTP → `kalamdb-api`. Dialect → `kalamdb-dialect`. CLI generate/build/deploy → `cli`. Config → `kalamdb-configs`. Metrics → `kalamdb-observability`. | PASS |
| **III. Minimal Dependency Expansion** | Two small crates (`arc-swap`, `smallvec`). Wasmtime is not a workspace dependency. No Deno/QuickJS/tsc-in-server. | PASS |
| **IV. Validation Ships Together** | Each stage has an executable gate (see Migration Strategy). Security, pool-leak, revision-pin, inline-override, and CLI e2e are required before complete. Docs: `docs/reference/sql.md`, ADR-021 amendment, CLI help; user-facing SQL/CLI also update `../kalamdb-skills` when those paths are reachable. | PASS |
| **V. Composable APIs** | One `runtime.d.ts` + `defineProcedure` shared by inline and project. `FunctionRuntime` trait is **minimal** so core does not know V8. Not a UI-framework concern. | PASS |

**Post-design re-check**: Data model keeps one module revision, catalog inline fields (not per-routine modules), in-memory active set, and serialization-crate ownership of FlatBuffer transfer. No second transaction coordinator. PASS.

## Project Structure

### Documentation (this feature)

```text
specs/034-schema-first-functions/
├── spec.md
├── plan.md              # This file
├── research.md          # Phase 0 decisions
├── data-model.md        # Phase 1 entities
├── quickstart.md        # Phase 1 validation
├── contracts/           # Phase 1 interfaces
│   ├── sql-create-procedure.md
│   ├── rest-functions.md
│   ├── cli-functions.md
│   ├── host-api.md
│   ├── build-manifest.md
│   ├── runtime-config.md
│   └── typed-errors.md
└── tasks.md             # Phase 2 (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
backend/crates/kalamdb-functions/
├── Cargo.toml                    # default = ["catalog"] only; no wasmtime
├── build.rs                      # Stage 7: optional startup snapshot (after measurement)
├── src/
│   ├── engine.rs                 # lanes, root lease, global memory, revision-keyed pool
│   ├── engine_config.rs          # conservative defaults + max_memory_mb / heap_soft
│   ├── active_set.rs             # NEW — ActiveFunctionSet types (or in core)
│   ├── runtime.rs                # NEW — FunctionRuntime trait (no V8 in the trait)
│   ├── invocation.rs             # root vs nested; drop extra clones
│   ├── host.rs                   # thin native host ops
│   ├── v8_adapter.rs             # isolate pool, cleanup, snapshot load
│   ├── v8_async.rs               # ABI v2 only
│   ├── convert.rs                # JS ↔ RoutineValue via serialization transfer codec
│   ├── wrap.rs                   # bootstrap implements runtime.d.ts; one ctx shape
│   ├── revision.rs
│   ├── activation.rs
│   ├── limits.rs                 # ABI_VERSION = 2; host-op limits
│   └── value.rs
backend/crates/kalamdb-serialization/src/
└── function_value.rs             # NEW — in-memory FlatBuffer transfer + zero-copy check
backend/crates/kalamdb-core/src/functions/
├── executor.rs                   # FunctionExecutionRoot; lookup via ActiveFunctionSet
├── host.rs                       # reentrant nested call; PrincipalSessionCache
├── runtime_state.rs              # ArcSwap<ActiveFunctionSet>; lazy engine
├── acl.rs                        # drop module_id_for(routine); anonymous explicit grant
└── call_types.rs                 # SmallVec frames; Arc<HeaderMap> HTTP
backend/crates/kalamdb-system/src/providers/catalog/
└── models/routines/              # inline_source_hash, inline_artifact_id
backend/crates/kalamdb-dialect/src/ddl/create_procedure.rs
backend/crates/kalamdb-handlers/crates/ddl/src/procedure/
backend/crates/kalamdb-api/src/http/functions.rs
backend/crates/kalamdb-configs/src/config/   # [functions.runtime]
backend/crates/kalamdb-observability/src/function_metrics.rs
cli/src/workflow/
├── functions.rs                  # real build, status, revisions, rollback, override, logs
├── deploy/                       # artifact rollout + dry-run local build
├── schema/typescript.rs          # runtime.d.ts + registry; no overwrite; no auto-override
└── project/config.rs             # [functions] path/runtime/module
benchv2/comparison/drivers/kalamdb_functions/
docs/architecture/decisions/adr-021-kalamdb-functions.md  # amend
docs/reference/sql.md
```

**Structure Decision**: Keep runtime in `kalamdb-functions` and orchestration in `kalamdb-core`. Do **not** add a new crate for the active set or HTTP context. Put the JS-boundary FlatBuffer codec in `kalamdb-serialization` so functions does not own persistence-format choices (ADR-021). CLI owns generate/build/bundle; the server never installs npm.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| `arc-swap` workspace dep | Cheap immutable publish of `ActiveFunctionSet` on every root | `RwLock<Arc<...>>` adds writer coupling on the invoke path; `watch` is for async notify, not a hot read |
| `smallvec` workspace dep | Frames and principal cache are almost always 1–3 entries | `Vec` is unbounded and clones more on nested DEFINER; persistent trees are overkill |
| `FunctionRuntime` trait | Spec requires `kalamdb-core` not to know V8 | Inlining `V8Session` in core blocks later runtime experiments without a rewrite |
| FlatBuffer transfer at JS boundary (vs ADR-021 “Arrow not bytes”) | Spec US24: zero-copy check / single encode; JSON is the thing to kill | Keep field-by-field `ScalarValue`↔V8 (current `convert.rs`) plus JSON for JSONB — still copies graphs and fights the memory budget |
| Startup snapshot in Stage 7 | Deno-style bootstrap reuse after measurement | Shipping snapshot complexity before Stage 0–3 memory work hides regressions |

No extra services, actors, or worker processes. No QuickJS/Wasmtime.

## Migration Strategy (Stability-First)

Follow spec Delivery Sequence. One stage, one gate. Preserve working foundations (CAS activation, contract snapshot, typed `EXECUTE`, invoker/definer, filestore artifacts).

| Stage | Deliverable | Regression gate |
|-------|-------------|-----------------|
| **0 Baseline** | Capture tests, function bench, idle/warm RSS, binary size; commit numbers under `benchv2/comparison/results/` (or feature notes) | Existing `kalamdb-functions` / core function tests green; numbers recorded in seconds |
| **1 Schema-first freeze** | Project-backed `CREATE PROCEDURE` without `AS 'src/...'`; LANGUAGE only with inline body; catalog inline hash/artifact fields; `ActiveFunctionSet` model; stop `module_id_for(routine_id)` as the project model | Dialect + catalog + parser tests; no production pool rewrite yet |
| **2 Context/security** | `FunctionExecutionRoot`, `ProcedureFrame` SmallVec, actor/principal, principal `ExecutionContext` cache, HTTP `HeaderMap`, typed errors, sensitive headers, shared `runtime.d.ts` surface aligned to `ctx` | Security tests: EXECUTE, anonymous deny, INVOKER/DEFINER nest, header block, typed HTTP status |
| **3 Runtime/admission** | Root `RuntimeLease`, reentrant nested invoke, workers `min(cpus,4)`, global memory, revision-keyed pool, idle eviction, drop `nested_reserve` as the common nested strategy | Memory tests: nested does not take a second slot; pool leak test; timeout destroys unclean isolate |
| **4 Project revision** | Real `functions build`, manifest, one module artifact, registry entrypoint, upload, CAS, pinned active set; JS boundary FlatBuffer encode/reuse | Build offline; missing/unknown export fail; activation refuses hash mismatch; no per-request filestore read |
| **5 Inline + override** | Inline JS executable; inline TS stored until project compile; project export overrides; rollback restores inline | Inline → deploy override → rollback e2e |
| **6 CLI UX** | `status` / `revisions` / `rollback` / `override` / deploy `--dry-run` actually builds; deploy rolls out artifact | CLI e2e path in spec US22 |
| **7 Snapshot (optional)** | Static Kalam bootstrap snapshot + code cache **if** Stage 3 baseline shows material init cost | No user-revision snapshots unless numbers justify (out of scope by default) |
| **8 Final gate** | Backend tests, CLI e2e, benches, RSS vs budget, binary size without Wasmtime, pool/revision tests | Unit tests alone are **not** complete |

### Must not survive (if replacement tests pass)

`module_id = routine_id` for project code; per-routine modules; nested `FunctionEngine::invoke` admission; nested worker hop; `max_active × max_heap` as the envelope; `with_effective_identity` + new `OnceCell` per host SQL; eager header `String` map; HTTP status via `message.contains`; REST success wrapper; generate-only build; rollback error stub; default `wasm-runtime`; ABI v1 `spawn_blocking` path.

### Must preserve

ContractSnapshot, generated scaffolds/registry (extended with `runtime.d.ts`), content-addressed artifacts, CAS activation, immutable revisions, process-global V8 `Once`, hard timeout / terminate / near-heap-limit, `RoutineValue` type id, EXECUTE ACL, INVOKER/DEFINER, transaction coordinator, `kalamdb-serialization` ownership of durable bytes, REST `/v1/functions/{namespace}/{procedure}`, topic-trigger tx semantics, lazy engine `OnceLock`.

## Phase 0: Outline & Research

**Output**: [research.md](research.md) — all technical choices resolved (no NEEDS CLARIFICATION).

Key decisions preview:

- Default module name `backend` from `[functions] module`; one `FunctionModuleId` for the project.
- Inline lives on `CatalogRoutine` (`language`, `body`, `inline_source_hash`, `inline_artifact_id`); not a module per routine.
- `ActiveFunctionSet` published with `arc_swap::ArcSwap`.
- Nested host call pushes a frame and invokes the registered JS function on the **same** `V8Session` / lease.
- Shared generated `.kalam/generated/runtime.d.ts` + contracts.ts; inline typecheck via shim.
- JS boundary: `kalamdb-serialization` FlatBuffer transfer; zero-copy `ArrayBuffer` when contract hash matches; else one encode.
- `ABI_VERSION = 2`; delete v1 spawn_blocking path after tests migrate.
- V8 is the only function runtime; Wasmtime is not in the workspace.
- Anonymous EXECUTE only with explicit `RoutineGrantee` for anonymous — **PUBLIC does not include anonymous**.
- V8 152 already exposes `SnapshotCreator` / `CreateParams::snapshot_blob` — no V8 upgrade for Stage 7.

## Phase 1: Design & Contracts

**Outputs**: [data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md)

## Documentation and ADR

- Amend [ADR-021](../../docs/architecture/decisions/adr-021-kalamdb-functions.md): WASM not in default binary; one project module; JS-boundary FlatBuffer transfer (Arrow/`RoutineValue` remains the SQL value model); ABI 2.
- Update `docs/reference/sql.md` for LANGUAGE-only-with-body, inline vs project, REST body contract.
- CLI help strings for build/revisions/rollback/override/dry-run.
- When `../kalamdb-skills` is available, update canonical SQL/CLI skill content; if out of workspace, call that out in the implementing task.

## Validation Strategy

Narrowest falsifying check first (constitution):

1. Parser/catalog unit tests (Stage 1)
2. Security + HTTP unit/integration (Stage 2)
3. Engine unit: nested lease, memory admission, pool isolation (Stage 3)
4. CLI `functions build` offline (Stage 4)
5. Inline override + rollback (Stage 5–6)
6. `benchv2` functions driver + RSS (Stage 0 and 8)

Full sweep `./scripts/test-all.sh` only at Stage 8, with a running server for CLI e2e.
