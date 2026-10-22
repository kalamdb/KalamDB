# Implementation Plan: Unified SDK Contracts

**Date**: 2026-10-18  
**Spec**: [spec.md](spec.md)  
**API contract**: [contracts/public-api.md](contracts/public-api.md)  
**Status**: Planned; runtime implementation and validation remain in [tasks.md](tasks.md)

## Summary

Ship Slice 1 before any Slice 2 work. Slice 1 keeps Drizzle as the only TypeScript table API and adds the thin Dart table builder in `kalam_link`, schema-qualified sync specs, full-versus-partial live decoding, one Drift connection (managed, app constructor, or attached), and the session/checkpoint/projection helpers Masky currently hand-writes. It rides the existing query, live, checkpoint, and action implementations.

Slice 2, after Slice 1 has shipped, is the shared contract manifest, generator API v2, **native typed function RPC with negotiated JSON/binary codecs and in-process typed V8 chaining**, live replay protocol, sync storage rewrite, and server idempotency. Do not start those in order to finish Slice 1.

The earlier Dart-only proposal is superseded where it would introduce different public method names or independent type/query/action machinery. Common surfaces are implemented and tested in pairs; Dart-only persistence is explicitly isolated.

## Technical context and ownership

| Area | Owner and likely source paths |
| --- | --- |
| Canonical contract types/names/metadata | `backend/crates/kalamdb-dialect/src/contracts/`; planner/schema catalog owners for semantic resolution |
| Generation orchestration | `cli/src/workflow/schema/gen.rs`, `load.rs`, `naming.rs`, `types.rs`, `procedures.rs`, `output.rs` |
| Language emitters | `cli/src/workflow/schema/typescript/`, `dart/`, `cli/templates/schema-gen/` |
| API exports | `cli/src/workflow/schema/api_exports/`; existing protobuf/FlatBuffers consumers must remain compatible |
| Function execution contracts | `backend/crates/kalamdb-api/src/http/functions.rs`, `routes.rs`, `backend/crates/kalamdb-core/src/functions/{executor,bind,convert,acl}.rs`, `kalamdb-functions` |
| Shared transport/materialization | `link/link-common/src/`, `link/sdks/rust`, `link/kalam-link-wasm`, `link/kalam-link-dart`, `kalamdb-serialization`; reuse canonical `LogicalTypeRef` and `RoutineValue` |
| TS core and live facade | `link/sdks/typescript/client/src/`, existing `live/descriptor.ts`, `controller.ts`, `projection.ts` |
| TS ORM and React | `link/sdks/typescript/orm/src/`, `react/src/`; keep framework concerns out of client |
| Dart base and remote table builder | `link/sdks/dart/link/lib/src/`; one model per file; regenerate FRB through its build workflow (no separate `kalam_orm` package) |
| Dart durable sync and custom actions | `link/sdks/dart/sync/`, `generator/` |
| Application validation | Disposable paired examples, then `/Users/jamal/git/masky-v3/app/lib/services/sync/` |
| Documentation | `docs/architecture/decisions/2026-08-16-kalam-sync-dart-package.md`, SDK READMEs/examples, `../KalamSite/content/sdk/**`, `../kalamdb-skills` and generated mirrors |

Use existing repository build profiles and dependency policy. No new runtime validation framework or new SDK package is required: generated codecs and Dart's remote table builder live in existing packages (`@kalamdb/client`, `@kalamdb/orm`, `kalam_link`, `kalam_sync`). Keep rich validation adapters optional. Do not turn this feature into a backend parser rewrite, general transaction API, or a new all-purpose ORM.

### Spec 037 handoff integrated into Spec 038

Spec 038 is the implementation owner for the generated SDK/native RPC path; it **consumes**, rather than supersedes, Spec 037's PostgreSQL-facing semantics and V8/value ABI contract:

| Existing 037 contract | Binding work in 038 | Guardrail |
| --- | --- | --- |
| [`functions-v8.md`](../037-pg18-compatibility/contracts/functions-v8.md), immutable `LogicalTypeRef` descriptors and typed `RoutineValue` ownership | Phase A descriptor/codecs, Phase B2 native JSON/binary calls and nested V8 value forwarding | No second type registry, codec framework, serializer, or independent function executor |
| [`functions-v8.md`](../037-pg18-compatibility/contracts/functions-v8.md), host views, ownership, cancellation and byte budgets | T058/T059 below, Rust/V8 and TS/Dart conformance | No blanket zero-copy; independently validate ownership, aliasing, projection, principal and lifetime |
| [`routines-postgresql.md`](../037-pg18-compatibility/contracts/routines-postgresql.md), signature binding/overloads/defaults/SQL CALL semantics | T052/T053/T060 descriptor revision and SQL/native parity | No SQL parser change or JSON envelope required for PostgreSQL SQL clients |
| Spec 037 `#393`, `#394`, `#397` evidence streams | Reuse upstream G3/G4 tests and record their status in Slice 2 gate | A checked box in 037 is not a substitute for observed 038 integration evidence |

Spec 037 owns PostgreSQL compatibility and its separate release gates. Spec 038 owns **generated TS/Dart invocation, transport negotiation, and integration into that ABI**. Any incompatible semantic change must be reconciled in both contracts with explicit versioning; do not quietly re-specify PostgreSQL behavior.

## Constitution check

| Principle | Application |
| --- | --- |
| Performance first | One manifest/type policy, immutable query compilation, shared materializer, bounded queues/retention, indexed scheduler, measured latency/memory |
| Boundary ownership | Transport/client, ORM, UI, and durable storage remain separate; backend file/RocksDB details remain in filestore/store |
| Minimal dependencies | Zero new SDK packages; `kalam_link` has no Drift dependency; `kalam_sync` uses Drift as its one local database; no TS persistence dependency is introduced |
| Tests and docs together | Every implementation slice has executable parity tests and paired docs; generator examples are compiled |
| Composable APIs | Same generated table/function/query descriptors serve ordinary calls, live views, UI hooks, and optional sync |
| Native RPC without duplicating execution | JSON and binary adapters bind canonical typed arguments once and invoke existing `FunctionService::invoke`; PostgreSQL SQL stays available |
| Security/transaction fidelity | Native and nested calls preserve EXECUTE, RLS, DEFINER/INVOKER, cancellation, transaction ownership and single-Raft-group limitation |

## Delivery phases

Implement Slice 1 to its gate, then stop. Phases A–F below are the full backlog; each phase is tagged with the slice that may start it. Slice 2 phases must not be pulled forward to unblock Slice 1.

### Slice 1 — Dart table access, one database, Masky glue

Record the public APIs that must keep compiling. Add the Dart table builder in `kalam_link` on today's `query` / `insert` / `update` / `delete` / `live` calls. Extend `kalam schema gen` only enough to emit schema-qualified `tableId`, `liveSql`, sync mode, and full-row plus partial-patch decoders. Keep the TypeScript Drizzle emitter as it is.

On the existing `kalam_sync` tables, add the three database modes, `checkpoints.seedIfAbsent`, `projections.reset`, coalesced session reconciliation, auth recovery, debounced connection state, and a retryable `KalamClient.init`. Point Masky at those APIs and delete only the glue they replace.

**Slice 1 gate**: existing TS client/ORM/React and Dart link/sync/generator suites stay green; a Dart table read/write/live example runs against the current server; Masky uses one SQLite connection and no longer hand-rolls the helpers above.

### Slice 2 — Protocol, manifest, storage, idempotency

Start only after the Slice 1 gate. This is Phases A, B, D, the storage rewrite in E, and the generator/perf work in F.

### Phase A — Establish semantic metadata and fixtures (Slice 2)

Record the current supported public API inventory and before-change examples. Extend authoritative metadata for real ordered keys, default/identity/generated fields, recursive types, routine signatures/cardinality, and capabilities. Implement stable names/entity hashes and one recursive codec policy before emitting new facades.

Create cross-language vectors under a new `link/tests/sdk-contracts/` fixture area. Each vector contains schema, typed/wire examples, expected errors, and live traces. Fixtures are data; target tests execute their own implementation against them. Reuse existing server/planner behavior rather than duplicating a third SQL interpretation in test code.

### Phase B — Generate and execute routines (Slice 2)

Split generated modules and add versioned manifests/atomic output publication. Add typed executor/error contracts to TS and Dart, decoding function results and honoring cardinality. Generate operations once and have immediate methods delegate to them. Add capability/contract discovery and version-aware HTTP operation export. **Immediate generated function methods use the native operation executor, not `client.query("SELECT ...")` or `CALL` text.** The legacy SQL path remains a separate compatibility interface.

### Phase B2 — Native typed RPC, JSON/binary transport and in-process V8 chaining (Slice 2; US7)

**Migrate the generated client first, not SQL parsing.** Inventory existing `POST /v1/functions/{namespace}/{procedure}`, `FunctionService::invoke`, `RoutineValue`, `ProcedureCatalog`, generated TS/Dart calls, 037 ABI tasks and the serialization crate. Make a canonical, revision-pinned routine descriptor (types, modes, ordinals, defaults, result/cardinality, ABI/codec version and permission dependencies) once, reused by HTTP, Rust, WASM, FRB and V8. Allow unknown/newer descriptors to fail before a mutation; pin in-flight revisions. Do not cache evaluated volatile defaults.

**JSON first:** Extend the existing route and shared link client with a versioned typed request/response path; preserve the unversioned JSON body for existing callers and preserve PostgreSQL SQL `SELECT` / `CALL`. Feed descriptor-bound values directly to `FunctionService::invoke` without synthesizing or parsing SQL. Generate both SDKs to call that path with checked codecs; retain compatibility aliases.

**Binary second:** Add a versioned binary envelope with operation signature and codec version, encoded by the existing canonical serialization/type owners. Reuse the same argument/result binder and execution path as JSON. Negotiate `auto`/`json`/`binary` once per compatible server/auth scope and choose a measured default; JSON must remain a full-featured fallback selected **before execution**. The two encodings do not represent different semantics. No new serializer, type enum, or redundant serialization through `serde_json::Value` for binary calls.

**Internal V8 boundary:** Reconcile with 037 `functions-v8.md` by passing native typed scalar/Arrow owners and explicit materialized JS values as appropriate. Eliminate redundant nested-call JSON tree round trips, preserve child isolate/revision/permission frames and bounded async lifetimes, and account for pinned Arrow buffers and external V8 memory. Zero-copy is an individually proven eligible boundary, not an API-wide claim.

**Gate B2:** Both generated SDKs call the same routine via direct JSON and binary RPC with lossless conformance; nested typed calls and SQL/native parity pass authorization, RLS, transaction, cancellation, ABI and revision tests. Cross-data-Raft-group writes explicitly fail atomicity; STREAM autocommit and external effects remain outside rollback guarantees. Measure p50/p95, CPU, allocations and wire bytes against generated SQL before selecting `auto` policy. Track T051–T063; do not advertise B2 prior to this evidence.

### Phase C — Add common queries and ORM adapters (Slice 1, then Slice 2)

Slice 1 keeps TypeScript table queries on Drizzle and the generated `kTable` schema, and adds the thin Dart remote builder on the current client. Slice 2 emits Drift mirror tables from the shared manifest and enriches named SQL queries through the canonical parsed/planned representation. Do not add a Rust ORM crate.

### Phase D — Align live execution and framework integration (Slice 2)

Extend the existing TS descriptor/controller rather than add another competing controller. Add equivalent Dart handles. Centralize protocol metadata, key reconciliation, snapshot completeness, typed parameter binding, and supported local projection semantics. Add shared leases/deduplication and safe cancellation generations. Both language bindings expose explicit acknowledgement, including TS where the ordinary callback API currently does not provide durable ack parity.

React overloads and Flutter examples consume the same descriptors/state. Raw SQL compatibility uses correct parser/binder behavior, retaining quoted identifiers and parameters. Remove old regex/rewrite code only when its callers migrate and matching server fixtures pass.

### Phase E — Complete durable behavior and server guarantees (database modes in Slice 1; storage rewrite in Slice 2)

Slice 1 supports a Kalam-managed database, a managed database built from the app's Drift constructor, and an already-opened app database, on the current checkpoint and action tables. Slice 2 migrates sync storage to query generations/membership, inbox/projection receipts, typed action results, stacked optimistic overlays, and worker fencing. Preserve existing queued work and identifiers. Distinguish mirror resnapshot from event replay gaps only in Slice 2. Same-database projections commit with the cursor; cross-database receipts remain only for a destination that cannot share the connection.

Implement explicit server idempotency capability before accepting auto-retryable generated calls. Use existing API/function service boundaries and appropriate store/raft ownership; coupling effects and receipt must share the authoritative transaction/consistency boundary. External-effect routines without equivalent guarantees remain non-durable/uncertain.

Expose reusable sync config, account/projection lifecycle, budgeted catch-up, and typed diagnostics. Share the existing client/socket per active account; avoid a second materializer for a persisted mirror. Make scheduling and status queries bounded and indexed.

### Phase F — Migrate applications and ship a usable workflow (Masky glue in Slice 1; the rest in Slice 2)

Slice 1 migrates Masky onto the new helpers and the one Drift connection, and updates the docs for those APIs. Slice 2 creates untouched TS and Flutter projects on generator API v2, evolves schema/routines, and runs the large-row performance gates. Measure generic glue reduction against the same file/category baseline.

Update docs and canonical skills; generate any mirrors. Publish migration examples and package compatibility requirements. Remove obsolete implementations only after legacy adapters and new APIs pass equivalent behavior tests.

## Validation strategy

| Gate | Required evidence |
| --- | --- |
| Generation | Golden semantic manifest, deterministic output, no partial overwrite, compiler/analyzer-positive and negative fixtures in both targets |
| Scalar/codec parity | Recursive arrays, domains, enum unknowns, large integers, decimals, bytes/files, temporal precision, SQL/JSON null, malformed required fields |
| Table/routine behavior | Composite keys, defaults/omission/null, immutable fields, single/list/void/set results, overload signatures, escaped identifiers, bound values |
| Live behavior | Same event traces -> same state; empty/multi-batch snapshot, partial patch, key-only delete, filter entry/exit, reconnect, errors, observer sharing, capacity |
| Transport | Real server + native Rust, WASM/browser, and FRB clients; no metadata dropped across bridges; old-server capability failures explicit |
| Sync durability | Process kill around every receipt/apply/ack/execute boundary, incomplete generation, failed external projection, outbox result recovery, codec migration |
| Ordering/ownership | Same-key retry cannot be overtaken, stacked overlays survive old echoes, foreground/background competing claims are fenced, account callbacks cannot leak |
| Server API | SQL/HTTP same routine semantics, authentication/authorization, forbidden context inputs, permission-safe manifests, idempotency reuse/conflict/expiry |
| Native function RPC | Generated TS/Dart calls bypass SQL parsing; identical typed results/defaults/errors in legacy SQL, v2 JSON and v2 binary via one FunctionService |
| V8 typed ABI and chaining | No nested SQL/HTTP/JSON detour, child-realm security and single-group transaction preserved; Arrow/buffer ownership, invalidation and memory-budget probes pass |
| Binary and fallback safety | Canonical lossless nested codecs, negotiated format versions, forged/stale/malformed inputs rejected; no automatic post-dispatch mutation fallback/retry |
| Product workflow | Untouched projects plus real Masky migration; no hand-edited generated files, truthful readiness/error output |
| Performance | 1k/10k/100k rows, cold/warm cache, sustained mutation stream, reconnect burst, large completed-action history, repeated attach/dispose |

Native invocation benchmarks must compare the same routine in generated SQL, direct typed JSON and direct binary plus two-step nested functions; include zero-argument, tiny scalars, large integers/decimals, composites, arrays and larger payloads. Measure parser/planner time, codec time, CPU, allocations, wire bytes, throughput, p50/p95 and V8/bridge crossing cost. Prefer JSON for small calls unless actual measurements justify binary. 

Performance runs use the same machine/profile/dataset, warm-up and at least five measured repetitions. Report median and p95 latency/runtime in seconds, peak/retained memory, decoded rows/bytes, local transactions, and socket/subscription counts. Compare before/after at each relevant path; explain any >10% regression. Memory should plateau under repeated attach/dispose and configured retention, rather than grow with total historical actions/events. Do not claim improvements without measurements.

## Suggested commands and execution discipline

Commands below identify existing test entry points. Revalidate package scripts as implementation changes; add the new focused fixtures to these suites.

```sh
cargo nextest run -p kalam-cli --lib -E 'test(workflow::schema)'
cargo check -p kalamdb-server
```

Run from each TS package: client `npm run build:ts`, `npm run test:types`, focused existing Node tests; ORM and React their package test/typecheck scripts. Some `npm test` scripts rebuild WASM, so batch feedback and prefer the narrowest relevant entry after a known build. Use dev-profile Rust bridge compilation; packaging scripts that select release profiles are not local iteration commands.

Run `flutter analyze` and focused `flutter test` suites in Dart `link`, `sync`, and `generator` packages. Compile real emitted files using supported minimum SDK/toolchain versions. Run browser tests only where the Dart package declares support; unsupported platforms must have explicit capability errors. Do not advertise a new Dart platform based only on generated web stubs.

For backend/CLI behavior changes, start an isolated server, build the CLI before e2e, then run required smoke tests before committing. Use `cargo nextest run` and repository test conventions. No live credentials, existing app data, production endpoints, or unrelated services are test fixtures.

## Migration and risks

- Generator v2 changes layout, public field casing, scalar types, and errors. Use one transitional release with explicit project opt-in and compatibility aliases; removal needs a documented breaking release.
- Existing TS Drizzle schemas have their own inferred shapes; conformance tests must compare generated models to ORM reads/writes, not merely snapshots of emitted strings.
- Complete replay, stable pagination, and idempotency are backend capabilities. When unavailable, expose unsupported/gap/uncertain outcomes; do not hide the dependency behind a client helper.
- Sync storage migrations must be restart-safe and preserve pending actions/receipts. Include rollback/recovery fixtures and bounded checkpoint rebootstrap.
- Native binaries/generated bridges already have unrelated local changes. Keep edits scoped and regenerate only after source changes are ready; inspect actual current diff before any overwrite.
- Do not silently retry/fallback an ambiguous function mutation after response loss or negotiation failure; a safe v2 idempotency receipt must be proven for any replayed effects.
- If staged writes span separate data Raft groups, the current transaction model rejects them. No automatic cross-group ACID or rollback for STREAM DML and external HTTP actions.
- Preserve the 037 ownership/read-view lifetime model: do not smuggle guest context or arbitrary V8 values across security realms, or label ordinary JS/network payloads zero-copy.
- v2 JSON and binary routing is an explicit negotiated upgrade. Old servers and legacy SDKs keep their existing route/shape; unknown versions fail before dispatch.
- External documentation/application paths may require the implementation environment's write permission. Produce and validate repository-side work first, then report the concrete remaining external edits if blocked.

## Completion boundary

Slice 1 is complete when its gate passes and the docs for those APIs match the code. Slice 2 remains required for the full feature and is complete only when every Slice 2 task and gate passes, including new Gate B2. A passing generator unit test, a written API sketch, or a successful connection alone is not completion of either slice. Do not treat Slice 1 as done by starting Slice 2.
