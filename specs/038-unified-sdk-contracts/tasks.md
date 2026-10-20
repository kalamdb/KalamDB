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

T001's Slice 1 scope is above. The remaining baseline work is Slice 2.
- [ ] T002 Add schema/type/query/routine and live-trace conformance fixture format plus TS/Dart harness entry points under `link/tests/sdk-contracts/` and existing target test directories. Covers FR-032.
- [ ] T003 Extend canonical metadata in `backend/crates/kalamdb-dialect/src/contracts/` for ordered actual keys, identity/generated/default policy, recursive type fidelity, routine overload/cardinality, and relevant entity versions; use authoritative AST/catalog/planner data. Covers FR-002, FR-004, FR-010, FR-011.
- [ ] T004 Implement shared stable naming, exact wire/public-name mapping, scoped entity hashes, manifest serialization/versioning, and collision tests in `cli/src/workflow/schema/` and contract owners. Covers FR-002, FR-007.
- [ ] T005 Define shared wire/codecs/errors in Rust client contracts and TS/Dart base SDKs, including large numbers, JSON/SQL null distinctions, temporal/file types, and error locations. Add positive/negative codec vectors. Covers FR-005, FR-006.

**Gate A**: Shared vectors and metadata are authoritative; no target invents a key/type/nullability policy.

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

**Gate B**: A schema-defined routine can be invoked through both generated clients and HTTP with equivalent typed results/errors, without handwritten serialization/registration.

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
- [ ] T050 [US6] Complete final cross-artifact/API/security review, confirm all success criteria with recorded evidence, and leave no unimplemented advertised examples or silent capability downgrades. Covers FR-001 through FR-034, SC-001 through SC-010.

## Dependencies and execution rules

- Slice 1 order is T001, T019, T018, T038, T045, T046, T048. Stop at the Slice 1 gate.
- Slice 2 order is A, then B, then the Slice 2 remainder of C, then D. T031–T033 start after T023–T027. T035 cannot complete before T034. T036 precedes overlapping background-worker claims.
- Slice 2 must not be used to finish a Slice 1 task.
- Do not check off a task based only on generated text or implementation presence. Record focused passing validation and any remaining capability limitations.
- Batch edits before checks, fix all errors from one captured output, use dev builds and `cargo nextest`, and follow local instructions for smoke/e2e tests.
- No runtime implementation is part of writing this specification. All proposed APIs require implementation and compiling examples before release.

## Requirement and success coverage

| Requirement group | Tasks | Acceptance |
| --- | --- | --- |
| FR-001 through FR-007 | T001-T011, T017, T044 | SC-001, SC-002, SC-006, SC-007 |
| FR-008 through FR-011 | T009, T015-T022, T028 | SC-001, SC-004, SC-006 |
| FR-012 through FR-016 | T023-T030 | SC-003, SC-005 |
| FR-017 through FR-025 | T031-T041, T045 | SC-003, SC-005, SC-009 |
| FR-026 through FR-028 | T008, T010, T012-T014, T042-T044 | SC-002, SC-004, SC-007 |
| FR-029 through FR-037 | T001, T018-T022, T030-T050 | SC-001, SC-004, SC-008, SC-009, SC-010 |
