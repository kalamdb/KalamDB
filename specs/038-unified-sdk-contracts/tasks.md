# Tasks: Unified SDK Contracts

Implement **Slice 1** to its gate before any Slice 2 task. Phase letters are backlog groups, not the execution order. Slice 1 is checked. Slice 2 stays unchecked. `[USn]` maps to the user stories in [spec.md](spec.md). Slice 1 uses today's wire format and generator output. Slice 2 signatures are in [contracts/public-api.md](contracts/public-api.md); Slice 2 transport and durability are in [contracts/protocol.md](contracts/protocol.md).

## Slice 1 — do these first

- [X] T001 [S1] Record the public APIs that must keep compiling, plus the Masky glue files Slice 1 replaces (`masky_kalam_specs.dart`, `kalam_decode.dart`, `projection_recovery.dart`, session reconcile / auth recovery / connection debounce in `kalam_sync_service.dart`). Preserve unrelated working-tree edits. Covers FR-029, FR-034, FR-037.
- [X] T019 [S1] Keep `kalam dev` / `kalam schema gen` emitting Drizzle `kTable` schemas and the existing driver, live table, file upload, conflict update, and `executeAsUser` behavior. Add a regression check that this output does not gain a second table API. Covers FR-009, FR-036.
- [X] T018 [S1] Add the thin remote table builder to `link/sdks/dart/link/` on the current `query`, `insert`, `update`, `delete`, and `live` calls. Emit schema-qualified `tableId`, `liveSql`, sync mode, and full-row versus partial-patch decoders. Do not wait for generator API v2, `createKalamFunctions`, or a new manifest. Covers FR-008, FR-009, FR-036, FR-037.
- [X] T038 [S1] On the existing checkpoint and action tables, add a Kalam-managed Drift database, an app Drift constructor on that connection, and an attached app database. Add coalesced `reconcileSession`, auth-error pause/resume, debounced connection state, retryable `KalamClient.init`, `checkpoints.seedIfAbsent`, and `projections.reset`. Do not migrate to generations, inbox, or leases. Covers FR-023, FR-035, FR-037.
- [X] T045 [S1] Point Masky at T018 and T038. Keep `masky.db` as the one Drift database. Delete only the glue those APIs replace. Leave domain mapping, REST handlers, and UI behavior in the app. Covers FR-029, FR-035, FR-037.
- [X] T046 [S1] Run the existing TS client/ORM/React and Dart link/sync/generator suites affected by T018/T019/T038. Covers FR-029, FR-037.
- [X] T048 [S1] Document the Slice 1 Dart table builder, database modes, and session helpers in the SDK README and the sync architecture note. Leave Slice 2 protocol and idempotency docs until that slice. Covers FR-033.

**Slice 1 gate**: those seven tasks pass, existing suites stay green, and no Slice 2 API is advertised.

## Slice 2 — after the Slice 1 gate

Do not start these to finish Slice 1. Server replay, a new sync schema, and idempotency receipts are this slice.

## Phase A — Baseline and shared semantics (Slice 2)

T001's Slice 1 scope is above. The remaining baseline work is Slice 2. Typed RPC and V8 ABI work below is an extension of Slice 2, not a reopening of checked Slice 1 tasks.
- [ ] T002 Add schema/type/query/routine and live-trace conformance fixture format plus TS/Dart harness entry points under `link/tests/sdk-contracts/` and existing target test directories. Covers FR-032.
- [ ] T003 Extend canonical metadata in `backend/crates/kalamdb-dialect/src/contracts/` for ordered actual keys, identity/generated/default policy, recursive type fidelity, routine overload/cardinality, and relevant entity versions; use authoritative AST/catalog/planner data. Covers FR-002, FR-004, FR-010, FR-011.
- [ ] T004 Implement shared stable naming, exact wire/public-name mapping, scoped entity hashes, manifest serialization/versioning, and collision tests in `cli/src/workflow/schema/` and contract owners. Covers FR-002, FR-007.
- [ ] T005 Define shared wire/codecs/errors in Rust client contracts and TS/Dart base SDKs, including large numbers, JSON/SQL null distinctions, temporal/file types, and error locations. Add positive/negative codec vectors. Covers FR-005, FR-006.
- [ ] T051 [US7] Inventory the live 0.8 JSON function endpoint, generated SQL wrappers, `FunctionService::invoke`, `RoutineValue`, serializer, native/WASM/FRB bridges, and [037 `functions-v8.md`](../037-pg18-compatibility/contracts/functions-v8.md) / [`routines-postgresql.md`](../037-pg18-compatibility/contracts/routines-postgresql.md); record exact implementation gaps and upstream #393/#394/#397 dependencies without claiming prior tasks complete. Covers FR-038, FR-047.

**Gate A**: Shared vectors and metadata are authoritative; no target invents a key/type/nullability policy.

## Mandatory binary consolidation work — five-stage ordered dependency chain (Slice 2; US8)

**Order is normative even when the task appears in a different phase**: harden (S) → lightweight shared codec (C0) → native RPC (B2) → new WS protocol + total MessagePack removal (W) → separately gated persistence/Raft (P). Existing T052–T063 require S and C0 before opening a public binary handler. None of these tasks are complete merely because this document exists.

### Stage 1 — Harden existing decoder (BEFORE sharing it with untrusted network clients)

- [ ] T064 [US8] Audit/fix `backend/crates/kalamdb-serialization/src/row/value.rs`: bound `decode_list` element count **before** `Vec::with_capacity`, check minimal/remaining encoded bytes and aggregate allocation budget, depth-limit `skip_value` during recursion and validate `decode_embedding` dimensions before reserve. Apply checked arithmetic to lengths/offsets and safe Arrow construction. Cover malformed lists, depth bombs, offset wrap and oversized count fixtures. FR-049, FR-058.
- [ ] T065 [US8] Harden all network decompression call sites. Replace unbounded `decompress_if_gzip` (`usize::MAX`) and failure-as-borrowed-data behavior in `link/link-common/src/compression.rs` and `link/kalam-link-wasm/src/helpers.rs`; ensure native, WASM, Dart, and server WS/HTTP binary paths enforce compressed/decompressed message size, expansion, time and cancellation budgets. FR-049, FR-057.
- [ ] T066 [US8] Audit and fix `NativeReadViewHandle` raw `Box::into_raw`/`unsafe` lifetime, finalizer/disposal, pin accounting, stale handle, cross-isolate/principal/projection leakage in `kalamdb-functions/src/abi/convert.rs` and allocator/host frames; prefer owned handles with explicit lease and checked generation over unchecked pointers. FR-054, FR-058.
- [ ] T067 [US8] Add libFuzzer/cargo-fuzz or equivalent targets and property/negative fixtures for envelopes, lists, recursion, Arrow offsets, optional compression, function values, WebSocket frames and canonical codecs; verify deterministic errors with strict resource budgets, no OOM/panic/stack overflow, no effect or privilege bypass. Run on native and WASM-relevant decoders. FR-049, FR-058.
- [ ] T068 [US8] **Gate S** security sign-off: versioned function schema checks, stream body limits independent of `Content-Length`, max input+decoded bytes, maximum counts/depth/work, V8 pin/lifetime tests and CI fuzz seeds. Record remediation and performance baseline before exposing any untrusted binary RPC/WS handler. Depends T064–T067. FR-049, FR-057/058.

### Stage 2 — Extract shared lightweight canonical codec

- [ ] T069 [US8] Feature-gate/refactor `kalamdb-serialization` so a lightweight codec core builds in `link/link-common` and browser `kalam-link-wasm` **without Arrow/DataFusion/RocksDB/server dependencies**; keep one canonical `LogicalTypeRef` type meaning and reuse existing scalar/ordinal binary encodings where compatible. If dependency cycles require a minimal internal crate, keep `kalamdb-serialization` as public owner, not a second protocol. FR-048, FR-050.
- [ ] T070 [US8] Implement one versioned KOBJ-family **network** framing/message-kind codec for function RPC and WS live, backed by the shared typed-value implementation. Share canonical encoding, null/presence, decimal/BigInt, timestamp precision, structured error, idempotency hash and strict decoder budgets across native Rust, backend, TS/WASM and Dart/FRB. Use distinct frame kinds, never arbitrary storage bytes accepted as a trusted RPC. FR-048, FR-051.
- [ ] T071 [US8] **Gate C0**: produce cross-runtime byte-identical golden/negative vectors, exact-type and memory/allocation tests; extend `scripts/check-serialization-boundary.py` CI to prohibit direct new WS/RPC `rmp_serde`, `flexbuffers` and duplicate JSON-based binary encoders in API/Link/WASM/FRB. Scoped documented exceptions for legacy durable/PGWire/Arrow IPC and explicit JSON external formats; benchmark wasm size and startup. Depends T068–T070. FR-048, FR-050, FR-056.

### Stage 3 — Typed native RPC

Existing tasks T052–T063 now have explicit prerequisites **T068 and T071** before T053/T056/T057 binary exposure and Gate B2. No `serde_json::Value` intermediate in the new binary transport; generated TypeScript/Dart call typed operation descriptors; `FunctionService::invoke` remains the only executor. Preserve safe v1 JSON compatibility. Measure rather than assume speed or zero-copy.

### Stage 4 — Converge WebSocket and REMOVE MessagePack COMPLETELY

- [ ] T072 [US8] Inventory `backend/crates/kalamdb-api/src/ws`, `backend/crates/kalamdb-commons/src/websocket*`, `link/link-common/src/connection`, `link/kalam-link-wasm/src`, `link/kalam-link-dart`, Dart generated clients and consumers for MessagePack codec/enum/negotiation/dep use; document old/current frame shape, subscriber scopes, compression, reconnection and server/SDK compatibility matrix. Depends T071/B2. FR-051, FR-052.
- [ ] T073 [US8] Implement bounded binary WebSocket server frames from the same Rust codec as RPC (distinct WS message kinds) for subscribe/auth control, snapshot begin/batch/end, event patch/put/delete, ack/cursor, error/reset and resume, maintaining RLS, auth scope, ordering, fanout/refcounts, bounded backpressure, server replay semantics and projection-specific authorization. No generic JSON/MessagePack tree roundtrip in new binary path. Depends T072 and B2. FR-051.
- [ ] T074 [US8] Implement WebSocket Kalam Binary parse/emit and identical typed event materialization through shared `link-common` core on native SDK, browser TS/WASM and Dart/FRB, eliminating duplicate `rmp_serde` and serde_json pass-through in new binary live path. Preserve event identity, snapshot completeness and cursor/ack parity. Depends T073. FR-048, FR-051.
- [ ] T075 [US8] Provide versioned authenticated WS capability negotiation, a **temporary isolated** legacy MessagePack adapter for a finite documented client/server upgrade window, and safe downgrade/upgrade errors. Old MessagePack must never be interpreted as Kalam Binary. Mixed-version/resume/ack/cancellation tests, streaming limits and fallback semantics are blockers. Depends T073/T074. FR-052.
- [ ] T076 [US8] Benchmark JSON/old MessagePack/new Kalam Binary WS under 1k/10k/100k row snapshots, 100k+ subscribers/high fanout, tiny/large patches, gzip, reconnect/ack and mixed versions; measure p50/p95, CPU, input/output bytes, allocations, retained/peak memory and per-field serialization. Prove specific eligible borrowed/Arrow views and account for copies across network/WASM/FRB. Depends T073–T075. FR-054/055.
- [ ] T077 [US8] After T075 compatibility window and validated cohort, **fully delete WebSocket MessagePack**: `SerializationType::MessagePack`, `serialization=msgpack`, direct `rmp_serde` calls, native Link/WASM/Dart WS branches, fallback, WS-only tests, docs and dependencies. Preserve `rmp-serde` only if proven necessary for independent durable/Raft legacy readers; new WS does not call it. Unknown/old clients receive explicit upgrade errors. Depends T075/T076. FR-052/056.
- [ ] T078 [US8] **Gate W**: real backend+native+browser+Flutter conformance and mixed-version upgrade tests, no WS MessagePack references outside clearly documented historical fixtures, pass static CI boundary scan, no lost snapshot/change/ack/replay/RLS event and no unexplained >10% perf regression. Record final removal commit and supported protocol versions. Depends T072–T077. FR-051/052/055/056.

### Stage 5 — Persistent and Raft formats independently

- [ ] T079 [US8] Inventory catalog/system FlexBuffers, persisted KOBJ row/stream, Raft protocol and snapshot MessagePack, PGWire and Arrow IPC; decide which *value encoding* can be shared and which physical framing/durable semantics must stay distinct. Do not conflate removal of WS MessagePack with historical disk/log decode. FR-053.
- [ ] T080 [US8] If a persisted/Raft encoding change is justified, implement durable versioned readers/writers, historical fixtures, rollback, corruption, rolling-upgrade/mixed-node, crash/recovery and schema-version isolation; preserve existing data. RPC/WS must not depend on this migration. FR-053.
- [ ] T081 [US8] **Gate P** separate release decision: measure disk/raft space, CPU, memory and upgrade risk against existing formats; retire legacy durable decoders only after proven data migration and compatibility. Do not block Gate B2 or W merely because P is deferred. FR-053.

## Phase B — US1 generated API and US5 routine contracts (Slice 2)

- [ ] T006 [US1] Add recursive codec emitters using the common semantic type tree in `cli/src/workflow/schema/typescript/` and `dart/`; reuse them in custom Dart action payload generation. Covers FR-004, FR-005, FR-006.
- [ ] T007 [US1] Split models/codecs/tables/operations/functions/queries/facade/optional adapters, introduce explicit generator API-version selection, and publish deterministic owned artifacts through staged output in `cli/templates/schema-gen/` and output orchestration; compile function-only imports without ORM/sync adapters and prove failure preserves prior output. Covers FR-002, FR-003, FR-029.
- [ ] T008 [US1] Generate typed operation and bound-call descriptors from `ProcedureCatalog`, preserving overload signature, defaults, parameter modes, result cardinality, and codec versions. Covers FR-004, FR-026.
- [ ] T009 [US1] Add `KalamExecutor` equivalents and typed get/getOne/getOptional/execute paths in TS client and Dart link; retain raw APIs and preserve structured errors. Covers FR-006, FR-008.
- [ ] T010 [US1] Generate paired `createKalamFunctions` and reusable `functions`/`operations` facade components with legacy TS namespace and Dart `KalamFunctions` adapters. This extends the Slice 1 Dart table builder; it does not block it. Covers FR-001, FR-026, FR-029.
- [ ] T011 [US1] Add compile-positive/negative generation fixtures for nested arrays/domains, input/output mismatch, composite keys, omission/null, and set/void/nullable results; assert ORM model agreement later. Covers FR-004 through FR-007, FR-032.
- [ ] T012 [US5] Add permission-safe SDK capability/contract endpoints in `kalamdb-api` with shared contract/service ownership, ETag caching, scoped compatibility, and no exposed private bodies. Covers FR-026, FR-028, FR-031.
- [ ] T013 [US5] Extend existing generated server handler contracts/registries and function invocation service to resolve typed operation identity consistently across SQL and HTTP; test authentication, overloaded/ambiguous routines, forbidden context, and result validation. Covers FR-026, FR-031.
- [ ] T014 [US5] Generate complete OpenAPI 3.1 operation paths, scalar/nested schemas, required/nullability/default rules, security and errors through `api_exports/openapi.rs`; preserve other exporters or version incompatibilities explicitly. Covers FR-027.

**Gate B**: A schema-defined routine can be invoked through both generated clients and HTTP with equivalent typed results/errors, without handwritten serialization/registration. Complete Gate B2 before claiming native transport performance or full 037 V8 parity.

### Phase B2 — US7 native routine RPC and native V8 values (Slice 2; after Phase A and T008/T009/T012/T013)

- [ ] T052 [US7] Resolve one immutable revision-pinned operation/call descriptor from canonical routine/type metadata (`LogicalTypeRef`, `ProcedureCatalog`, signature identity, argument names/modes/ordinals/defaults, result/cardinality, ABI/codec version and security dependencies). Generate matching TS/Dart descriptors; invalidate on DDL/type/grants/revisions and reject stale callers before effects. Depends on T003–T005, T008, T012. Covers FR-038, FR-040, FR-042.
- [ ] T053 [US7] Extend existing `POST /v1/functions/{namespace}/{procedure}` and shared Rust/link client with an explicit v2 descriptor-bound **typed JSON** invocation, dispatching directly to `FunctionService::invoke` without generated SQL or SQL parser use. Keep legacy route/body and SQL `CALL`/`SELECT` semantics intact; reject client-supplied privileged context. Depends on T052, T013. Covers FR-038, FR-039, FR-043, FR-044.
- [ ] T054 [US7] Replace generated TypeScript/Dart `client.query(\"SELECT ...\")` and `CALL` wrappers with typed `execute/invoke(operationDescriptor, input, options)` calls against the direct v2 JSON function path. Emit checked codecs and preserve documented old SDK aliases/version opt-in; compile generated clients against real SDK APIs. Depends on T053, T006–T010. Covers FR-038, FR-040, FR-047, SC-011.
- [ ] T055 [US7] Add canonical JSON/typed-value parity tests for NULL versus JSON null, missing/default/volatile values, INT64, exact NUMERIC, sub-millisecond timestamps, BYTEA, FILE, enum/domain, multidimensional/nullable arrays, composites, scalar/void/set results and malformed wire. Execute TS, Dart, native Rust, browser WASM and FRB where supported; no unchecked `as Result` and no lossy conversion. Depends on T052–T054. Covers FR-039, FR-040, SC-012.
- [ ] T056 [US7] Define and implement a versioned **binary routine request/response envelope** using existing `kalamdb-serialization` / canonical `LogicalTypeRef` descriptors and codec owner, not a second serializer/type registry. Cover minimal routing identity, signature/codec version, positional typed values, framing/limits and structured errors; bind directly to `RoutineValue` without passing through JSON. Depends on T052/T055. Covers FR-039, FR-040, FR-046.
- [ ] T057 [US7] Expose binary transport in Rust/link, TypeScript/WASM and Dart/FRB clients using the same conformance vectors. Implement `auto`/`json`/`binary` with scope-aware capability caching; default selection is evidence-based, and fallback happens only when non-execution is known or a proven idempotency receipt protects it. Keep legacy JSON clients compatible. Depends on T056/T012. Covers FR-039, FR-044, SC-012/SC-013.
- [ ] T058 [US7] Make nested `ctx.functions.<schema>.<method>` invocation pass typed native values in-process under the same root transaction/cancellation/deadline; eliminate redundant `pack_named_call_input` JSON stringify/parse, preserve child-realm/EXECUTE/INVOKER/DEFINER/RLS checks, validate typed return values and pin revisions. Do not add a second HTTP or SQL call. Reconcile with 037 V8 contract. Depends on T052/T055. Covers FR-041, FR-043, SC-012/SC-013.
- [ ] T059 [US7] Evolve the existing `RoutineValue`/V8 host path to preserve eligible Arrow owners/read views or explicitly transferred exclusive buffers with bounds, owner/generation/projection validation, child-realm rewrapping and byte budgets. No blind zero-copy claims or untracked pins; test cancellation, cross-await lifetime, aliasing and disposal. Coordinate with 037 #393/#394. Depends on T058 and approved ABI mapping. Covers FR-041, FR-046.
- [ ] T060 [US7] Add real-server SQL-vs-v2-JSON-vs-binary/nested parity and fault tests: EXECUTE grants, RLS, SECURITY INVOKER/DEFINER, forbidden context, signature/version mismatch, malformed frames, cancellation/timeout, rollback/topic staging, cross-data-Raft-group rejection, STREAM autocommit, unknown retry outcomes and external effects. Reuse 037 tests, do not duplicate their owners. Depends on T053–T059. Covers FR-040–FR-044, SC-011–SC-013.

**Gate B2**: Both generated SDKs call without synthesized SQL; JSON/binary and native nested typed calls share execution/security/transaction semantics. Codec and ABI/lifetime tests pass; any 037 implementation gaps remain tracked blockers, not assumed complete. Performance and docs finish in Phase F tasks T061–T063.

## Phase C — US2 common query and ORM experience (Slice 2 except T018 and T019)

- [ ] T015 [US2] Define immutable query plans/descriptors, typed predicates, parameter binding, cardinality, request deadline/cancellation, and capability checking in base SDK/shared client owners. Covers FR-008, FR-010, FR-011.
- [ ] T016 [US2] Add named-query input/configuration and canonical SQL analysis in `cli/src/workflow/schema/` with typed result/key/live metadata; reuse parsed/planned representation. Covers FR-010, FR-011.
- [ ] T017 [US2] Generate read/insert/patch/key models and typed column/table descriptors in both targets; prohibit key guessing, read-only writes, absent/null conflation, and incompatible generated inputs. Covers FR-004, FR-007, FR-008.
T018 and T019 are Slice 1 and are listed above. Do not expand them here into generator API v2.
- [ ] T020 [US2] Generate Drift mirror tables for the one session database, with explicit remote/local query separation and no business-model duplication. Apps include those tables plus their own tables in one Drift schema. Covers FR-009, FR-019, FR-035.
- [ ] T021 [US2] Add real-server paired CRUD/default/key/quoting/type tests, including generated model versus Drizzle/Drift shape checks. Covers FR-004, FR-005, FR-008, FR-011, FR-032.
- [ ] T022 [US2] Remove obsolete quote/default/file SQL rewrites only where replacement binder/provider behavior passes semantic tests; preserve explicit typed upload support. Covers FR-011, FR-029.

**Gate C**: Both languages demonstrate the same typed query/CRUD use cases; existing Drizzle applications still work.

## Phase D — US3 live parity and UI adapters (Slice 2)

- [ ] T023 [US3] Extend server/shared client delivery contracts with snapshot begin/end, delivery identity, scope/domain/history cursor, typed parameters, and reset/gap outcomes; version/capability-gate old servers. Covers FR-012, FR-013, FR-016.
- [ ] T024 [US3] Carry all metadata through WASM and FRB bridges; expose TS explicit acknowledgement corresponding to Dart's durable mode, and eliminate guessed row-max progress for new contracts. Regenerate generated bridges through supported scripts. Covers FR-016.
- [ ] T025 [US3] Align typed descriptors and live handles around the existing TS controller; add Dart counterpart, gap-free seeded state, cancellation generations, and idempotent disposal. Covers FR-012, FR-014.
- [ ] T026 [US3] Implement account/query/codec-aware shared subscription leases and deterministic teardown; test overlapping handles and changed parameters/auth scopes. Covers FR-014, FR-023.
- [ ] T027 [US3] Implement exact snapshot/patch/delete/filter-exit reconciliation and deterministic local ordering/window rules in shared owners; enforce explicit capacity failures instead of silent eviction. Covers FR-013, FR-015, FR-030.
- [ ] T028 [US3] Replace generated-query regex SQL preparation with structured descriptor compilation; route raw SQL preparation through the canonical parser and preserve string/identifier semantics. Covers FR-011, FR-015.
- [ ] T029 [US3] Extend existing React hooks/providers to accept query descriptors and expose common snapshots; add Flutter handle examples/bindings without a second state machine. Covers FR-001, FR-012.
- [ ] T030 [US3] Run shared trace tests and real-server native/WASM/FRB tests for empty snapshots, partial rows, key-only deletes, filter transitions, top-N promotion, reconnect/domain mismatch, listener sharing, and error propagation. Covers FR-012 through FR-016, FR-032.

**Gate D**: Same supported live query produces equivalent rows/completeness/errors in both SDKs, including failure/reconnect traces.

## Phase E — US4 Dart durability and US5 retry safety (Slice 2 except T038)

- [ ] T031 [US4] Add restart-safe Drift migration for query definitions/generations/membership, inbox/projection receipts, action results/overlays, and worker leases on the one session database; preserve old pending actions/IDs and quarantine unverified legacy cursors. Export migration steps so an app schema can compose them. Covers FR-017 through FR-023, FR-029, FR-035.
- [ ] T032 [US4] Implement `mirror` over the shared query descriptor with cached-first state, independent query membership, atomic generation activation, empty resnapshot, and local paged/delta views. Covers FR-017, FR-025, FR-030.
- [ ] T033 [US4] Implement durable consumers, explicit outcomes, same-database projection transactions, foreign-database receipts, replay-origin metadata, bounded retention, and public projection reset/rebuild. Move network effects into actions outside local transactions. Covers FR-018, FR-019, FR-025, FR-035.
- [ ] T034 [US5] Implement server idempotency capability/receipts at the authoritative transaction boundary, scoped input-digest conflicts, result replay, expiry detection, and safe external-effect rejection. Covers FR-020, FR-021, FR-031.
- [ ] T035 [US4] Generate eligible durable call descriptors and add typed action handles/results/restoration with codec migration, explicit unknown outcomes, and cancellation semantics; ordinary unsafe calls cannot enter automatic retries. Depends on T034. Covers FR-020, FR-021.
- [ ] T036 [US4] Implement persistent worker fencing, strict per-key unresolved-action ordering, dependencies, stacked optimistic overlays, and separate accepted/reconciled state. Covers FR-022, FR-023.
- [ ] T037 [US4] Refactor custom Dart action generation to shared operation/input/result codecs; retain typed application-defined REST actions with explicit retry policy. Covers FR-003, FR-020.
T038 is Slice 1 and is listed above. Catch-up budgets, fencing, and the storage migration stay in this phase.
- [ ] T039 [US4] Implement global/per-consumer catch-up budgets, explicit completion/hasMore/error reporting, safe cancellation/drain, and deadline-aware optional outbox flushing. Covers FR-024.
- [ ] T040 [US4] Replace full-history watchers/scans with indexed bounded claims, aggregates, safe cleanup, transaction batching, and decode reuse; expose typed account-scoped diagnostics. Covers FR-025, FR-030.
- [ ] T041 [US4] Run process-kill, same-database projection, foreign-database receipt retry, invalid codec, replay gap, empty rebuild, duplicate delivery, foreground/background overlap on one connection, account switch, same-key ordering, and idempotency-expiry tests. Covers FR-017 through FR-025, FR-032, FR-035.

**Gate E**: Durable behavior survives every receipt/apply/ack/execute crash boundary or returns an explicit unrecoverable/uncertain outcome.

## Phase F — US6 workflow, Masky, documentation, and performance (Slice 2 except T045, T046, T048)

- [ ] T042 [US6] Expose T007's versioned generation in CLI/config/templates, add `schema gen --check`, and finish safe owned-file cleanup and consistent manifest export placement; exercise staged publication across multiple targets and failure recovery. Covers FR-002, FR-028, FR-029.
- [ ] T043 [US6] Update new-project TS/Dart templates and existing dev/watch pipeline for paired generation, typed server handlers, truthful schema/activation/app status, and actionable diagnostics. Covers FR-026, FR-028.
- [ ] T044 [US6] Turn every example in `quickstart.md` and API contract into compiling/running fixture tests; add an untouched paired disposable-project workflow including schema/routine evolution and framework health. Covers FR-001, FR-028, FR-032.
T045 and T046 Slice 1 scopes are listed above. Broader Masky mirror migration and backend smoke for Slice 2 stay with T047.
- [ ] T047 [US6] Benchmark paired 1k/10k/100k row workloads, reconnect, large action history, cache startup, and attach/dispose retention; record each relevant runtime in seconds and explain regressions against baseline. Covers FR-030, SC-008.
T048 Slice 1 scope is listed above. The parity/platform matrix for Slice 2 lands with T049.
- [ ] T049 [US6] Align TS cohort and Dart package constraints, generator/runtime/server compatibility checks, and version verification; remove obsolete duplicate implementations only after adapters pass. Covers FR-003, FR-028, FR-029, FR-033.
- [ ] T050 [US6] Complete final cross-artifact/API/security review, confirm all success criteria with recorded evidence, and leave no unimplemented advertised examples or silent capability downgrades. Covers FR-001 through FR-058, SC-001 through SC-020.
- [ ] T061 [US7] Benchmark generated SQL `SELECT`/`CALL` vs direct JSON vs direct binary using identical bodies and native/WASM/FRB paths; include zero-arg, scalars, INT64/DECIMAL, large composites/arrays, nested calls, warm/cold p50/p95, throughput, CPU, allocations, parser/codec overhead and wire bytes. Set `auto` preference from reproducible evidence; no speed/zero-copy claims without measurements. Depends on T055–T060. Covers FR-045, SC-014.
- [ ] T062 [US7] Reconcile and publish native RPC/ABI documents across `spec.md`, `plan.md`, `tasks.md`, `data-model.md`, `contracts/protocol.md`, `contracts/public-api.md`, quickstart, generated API docs and OpenAPI. Explicitly link 037 normative dependencies, v1/v2 compatibility, single-group transactions and STREAM/external effects; generate accurate examples only after tests. Depends on T060. Covers FR-047, SC-011–SC-013.
- [ ] T063 [US7] Final two-SDK/native transport release gate: verify Gate B2, no duplicate serializer/executor, stale/revision and unsafe retry behavior, lifecycle/memory probes, reproducible perf records and correct 037 ownership. Preserve legacy coverage; mark completion only with recorded evidence. Depends on T060–T062. Covers FR-038–FR-047, SC-011–SC-014.

## Dependencies and execution rules

- Slice 1 order is T001, T019, T018, T038, T045, T046, T048. Stop at the Slice 1 gate.
- Slice 2 SERIALIZATION order is mandatory: T064–T068 (Gate S) → T069–T071 (Gate C0) → RPC T052–T063 (Gate B2) → WS T072–T078 (Gate W) → independently deferred storage/Raft T079–T081 (Gate P). Non-serialization A/B/C/D/E/F work remains separately sequenced. T051 source inventory starts immediately. Both T053 direct binary endpoint preparation and T056/T057 public binary require T068/T071. Gate W must include full WS MessagePack removal, not merely new-format support. T061–T063 finish the RPC gate in F. T052 waits for canonical metadata/T008/T012, T053 for T052/T013, T054 for typed JSON/T006–T010, T056 for codec parity, T057 for binary/capabilities, and T058/T059 for approved 037 typed ABI mapping. T031–T033 start after T023–T027. T035 cannot complete before T034. T036 precedes overlapping background-worker claims.
- Slice 2 must not be used to finish a Slice 1 task.
- Do not check off a task based only on generated text or implementation presence. Record focused passing validation and any remaining capability limitations.
- Batch edits before checks, fix all errors from one captured output, use dev builds and `cargo nextest`, and follow local instructions for smoke/e2e tests.
- No runtime implementation is part of writing this specification. All proposed APIs require implementation and compiling examples before release.
- 037 PostgreSQL routine/ABI semantics remain normative dependencies. Use [037 `functions-v8.md`](../037-pg18-compatibility/contracts/functions-v8.md) and [`routines-postgresql.md`](../037-pg18-compatibility/contracts/routines-postgresql.md) for type and execution meanings; 038 owns SDK transport/codegen implementation and verification, not a second execution engine.

## Requirement and success coverage

| Requirement group | Tasks | Acceptance |
| --- | --- | --- |
| FR-001 through FR-007 | T001-T011, T017, T044 | SC-001, SC-002, SC-006, SC-007 |
| FR-008 through FR-011 | T009, T015-T022, T028 | SC-001, SC-004, SC-006 |
| FR-012 through FR-016 | T023-T030 | SC-003, SC-005 |
| FR-017 through FR-025 | T031-T041, T045 | SC-003, SC-005, SC-009 |
| FR-026 through FR-028 | T008, T010, T012-T014, T042-T044 | SC-002, SC-004, SC-007 |
| FR-029 through FR-037 | T001, T018-T022, T030-T050 | SC-001, SC-004, SC-008, SC-009, SC-010 |
| FR-038 through FR-044 | T003-T010, T012-T013, T051-T058, T060 | SC-011, SC-012, SC-013 |
| FR-045 through FR-047 | T051-T063 | SC-011, SC-012, SC-013, SC-014 |
| FR-048 through FR-050, FR-057-FR-058 | T064-T071 | SC-015, SC-016 |
| FR-051, FR-052, FR-054-FR-056 | T070-T078 | SC-015, SC-017, SC-018, SC-019 |
| FR-053 | T079-T081 | SC-020 |
