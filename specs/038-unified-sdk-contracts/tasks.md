# Tasks: Unified SDK Contracts, Fresh Epoch, and Measured Codec

**Revised:** 2026-10-24. **Status:** Ready for `/speckit-implement`. All architectural decisions and owner approvals are recorded in [spec.md](spec.md) and [plan.md](plan.md). Completed foundations are marked `[X]`; the remaining ready-to-implement tasks (`T052`, `T057`, `T077`, `T071`, `T098`, `T110`, `T050`) are organized into ordered execution phases below with exact file paths and test commands.

## Completed Foundations (Verified in Tree)

### Slice 1 & Gate A — Canonical Metadata, Naming, and Unified Conversions

- [X] T001 [S1] Record the SDK surface and Masky generic-glue baseline; retain unrelated working-tree edits.
- [X] T019 [S1] Emit the TS Drizzle `kTable` schema and driver/live/file/conflict-update behavior without a second table API.
- [X] T018 [S1] Add the thin Dart remote table builder, qualified `tableId`/`liveSql`/sync metadata and full-row/partial-patch decoding.
- [X] T038 [S1] Add managed, app-constructed and attached single-Drift modes, session reconciliation, auth recovery, debounced state, retryable initialization and checkpoint/projection helpers.
- [X] T045 [S1] Adopt those helpers in Masky while retaining domain/REST/UI responsibilities and one local database connection.
- [X] T046 [S1] Record the affected TS/Dart package regression validation for the original Slice 1.
- [X] T048 [S1] Document those Slice 1 APIs and sync architecture.
- [X] T002 Establish shared schema/type/routine/live/adversarial fixtures and native/TS/Dart harness entry points under `link/tests/sdk-contracts`. FR-032/062.
- [X] T003 Extend catalog/contract metadata in `backend/crates/kalamdb-dialect/src/contracts/snapshot.rs` and `compile.rs` for real ordered keys, defaults/identity/generated fields, recursive types, routine modes/overloads/cardinality and entity revisions. FR-002/004/010/011.
- [X] T004 Implement deterministic names, exact SQL/public-name mapping, `ScopedContractHashes`, and current-epoch manifest in `backend/crates/kalamdb-dialect/src/contracts/hash.rs` and `cli/src/workflow/schema/naming.rs`. FR-002/007.
- [X] T005 Unify `DomainValue`, `DomainFileRef`, `domain_value_to_scalar`, `scalar_to_domain_value`, and ISO temporal/decimal/JSON scalar conversions in `backend/crates/kalamdb-commons/src/models/domain_value.rs` and `backend/crates/kalamdb-commons/src/conversions/scalar/{domain.rs,json.rs}`. FR-005/006/040.
- [X] T051 Inventory execution/binder/serialization/Link/WASM/FRB paths and external format boundaries in `link/tests/sdk-contracts/t051-inventory.md`. FR-047/048.

### Gate S & Gates C0/D — Hardened Decoders, Lean Core, and Approved `KDOM` Codec

- [X] T064 [P0] Bound untrusted preallocation, recursive `skip_value`, embedding dimensions, and parsing-time depth/work/element/allocation budgets in `backend/crates/kalamdb-serialization/src/row/{value.rs,batch.rs}`. FR-049/058.
- [X] T065 [P0] Enforce finite compressed/output limits and remove failed-decode plaintext fallback in `backend/crates/kalamdb-api/src/ws/compression.rs`, `link/link-common/src/compression.rs`, and `link/kalam-link-wasm/src/helpers.rs`. FR-057.
- [X] T066 [P0] Replace raw V8 read handles with the bounded generation-fenced `ReadViewRegistry` in `backend/crates/kalamdb-functions/src/abi/convert.rs`. FR-046/054/058.
- [X] T067 [P0] Add negative/budget/truncation/skip/handle tests across `kalamdb-serialization` and `kalamdb-functions`. FR-032/049/058.
- [X] T068 [P0] Record Gate S hardening evidence in `link/tests/sdk-contracts/gate-s-evidence.md`. FR-049/057/058.
- [X] T069 Extract lean `kalamdb-serialization` core (`default-features = false`) without Arrow, DataFusion, RocksDB, HTTP server, or V8 dependencies. FR-048/050.
- [X] T082 Benchmark `KDOM` against JSON (`domain_function_payload_beats_json_on_the_comparison_mark` in `benchv2/src/comparison.rs`, 67% faster) and approve `KDOM` as the single production domain codec per Owner Decision 1 (`link/tests/sdk-contracts/gate-d-codec-selection.md`). FR-055/059/062.
- [X] T070 Implement `KDOM` value/framing codec (`backend/crates/kalamdb-serialization/src/domain.rs` and `ws_domain.rs`) with explicit widths, presence states (`Omitted`, `SqlNull`, `JsonNull`, `Void`), `EndpointSurface` allowlists, and `canonical_invocation_digest`. FR-048/051/059.

### Generated Contracts, Native RPC, Table/Query Analysis, and WebSocket `KDOM` Cutover

- [X] T006 Generate typed model codecs and descriptors from `ContractSnapshot` in `cli/src/workflow/schema/`. FR-004/005/006.
- [X] T007 Emit current-epoch TS/Dart/Rust schema/client artifacts in `cli/src/workflow/schema/gen.rs`. FR-002/003/029.
- [X] T008 Generate immutable `OperationDescriptor` with `epoch`, `codecVersion`, `routineId`, `signature`, `schema`, `name`, and `cardinality` in `cli/templates/schema-gen/typescript/src/generated/kalam.ts.template`. FR-004/026.
- [X] T009 Implement `KalamDBClient.invokeRoutine` (`link/sdks/typescript/client/src/client.ts`) and `KalamClient.invokeRoutine` (`link/sdks/dart/link/lib/src/kalam_client.dart`). FR-006/008/038.
- [X] T010 Generate `createKalam` and `createKalamFunctions` facades in TS and Dart without SQL fallback. FR-001/026/029.
- [X] T011 Validate generated positive/negative cases for nested types, arrays, non-`id` primary keys, file columns, and routines in `cli/src/workflow/schema/{generate_tests.rs,typescript/mod.rs,dart/mod.rs}`. FR-004-FR-007/032.
- [X] T012 Implement authorized `/v1/api/capabilities` and `/v1/api/contracts` endpoints with private ETag caching and epoch checks in `backend/crates/kalamdb-api/src/http/functions/{capabilities.rs,contracts.rs,error.rs}`. FR-026/028/031/061.
- [X] T013 Bind catalog routines across SQL and `/v1/functions/{namespace}/{procedure}` with rejected context keys and revision checks in `backend/crates/kalamdb-api/src/http/functions/invoke.rs`. FR-026/031/043.
- [X] T014 Generate typed OpenAPI 3.1 document (`BTreeMap` schemas/paths) at `/v1/api/openapi.json` in `backend/crates/kalamdb-api/src/http/functions/openapi.rs`. FR-027.
- [X] T053 Dispatch `application/x-kalam-domain` (`KDOM`) and typed JSON envelopes on `/v1/functions/{namespace}/{procedure}` in `backend/crates/kalamdb-api/src/http/functions/invoke.rs`. FR-038/039/043/044.
- [X] T054 Switch generated TS and Dart routines to `invokeRoutine` and remove `SELECT`/`CALL` SQL wrapper transport in `cli/templates/schema-gen/{typescript,dart}` and `cli/src/workflow/schema/{typescript,dart}`. FR-038/040.
- [X] T055 Test wide `Int64`, `Decimal128`, `SqlNull` vs `JsonNull`, and empty typed/untyped arrays in `backend/crates/kalamdb-core/src/functions/convert.rs` and `backend/crates/kalamdb-commons/src/conversions/scalar/`. FR-040/032.
- [X] T056 Delegate `domain_value_to_routine_value` and `routine_value_to_domain_value` through `kalamdb-commons::conversions::{domain_value_to_scalar,scalar_to_domain_value}` without JSON/MessagePack intermediates. FR-039/040/048.
- [X] T058 Reuse `FunctionValueEncoder` buffer and in-process `RoutineValue` passing for nested calls in `backend/crates/kalamdb-core/src/functions/`. FR-041/043.
- [X] T060 Verify HTTP function invocation, epoch mismatch, stale revision, and rejected context keys in `backend/crates/kalamdb-api/src/http/functions/invoke.rs`. FR-040-FR-044.
- [X] T061 Record RPC `KDOM` vs JSON latency comparison in `benchv2/src/comparison.rs` and `benchv2/comparison/drivers/kalamdb_functions/`. FR-045/055/062.
- [X] T062 Update WebSocket and ADR documentation in `docs/architecture/websocket-server.md` and `docs/architecture/decisions/2026-10-24-spec-038-acceptance-reconciliation.md`. FR-033/047.
- [X] T063 Approve RPC architecture and `KDOM` transport per Owner Decision 1. FR-038-FR-047.
- [X] T015 Implement `TypedQueryPlan`, cardinality, cancellation, and feature checking in `backend/crates/kalamdb-dialect/src/contracts/query_plan.rs`. FR-008/010/011.
- [X] T016 Analyze named SQL queries with canonical `KalamParser` AST and live eligibility in `backend/crates/kalamdb-dialect/src/contracts/query_plan.rs`. FR-010/011.
- [X] T017 Implement `table_row_contract` and `reject_forbidden_write` in `backend/crates/kalamdb-dialect/src/contracts/table_access.rs` and wire into `cli/src/workflow/schema/gen.rs`. FR-004/007/008.
- [X] T020 Retain single-session Drift table specs and `KalamTableSpec` generation in `cli/src/workflow/schema/dart/`. FR-009/019/035.
- [X] T021 Verify generated TS/Dart/Rust contract agreement and primary-key rules in `cli/src/workflow/schema/generate_tests.rs`. FR-004/005/008/011/032.
- [X] T022 Preserve typed `FILE` upload and parameter binding without hot-path SQL rewrite passes. FR-011.
- [X] T023 Encode/decode `WsSnapshotBegin`, `WsSnapshotBatch`, `WsSnapshotEnd`, `WsChanges`, `WsAck`, `WsCursor`, `WsError`, and `VersionDomain` in `backend/crates/kalamdb-serialization/src/ws_domain.rs`. FR-012/013/016.
- [X] T024 Carry `VersionDomain` and `BatchControl` through `kalamdb-serialization::ws_domain`, `link-common`, `kalam-link-wasm`, and `kalam-link-dart`. FR-016.
- [X] T025 Preserve live subscription state machine across `link-common`, TS, and Dart handles. FR-012/014.
- [X] T026 Multiplex shared WebSocket subscriptions per connection in `link/link-common/src/connection/shared/`. FR-014/023.
- [X] T027 Preserve insert/update/delete row and `old_values` deltas in `ws_domain.rs` and `SharedChangePayload`. FR-013/015/030.
- [X] T028 Use AST-based `QueryParser::analyze_subscription_query` and `analyze_named_query` without regex query parsing. FR-011/015.
- [X] T029 Bind React hooks (`@kalamdb/react`) and Dart `kalam_sync` streams to the shared client live lifecycle. FR-001/012.
- [X] T030 Verify domain WebSocket auth, change delivery, empty array/null/object cell preservation, and JWT subprotocol auth in `backend/crates/kalamdb-api/src/ws/mod.rs` and `ws_domain.rs`. FR-012-FR-016/032.
- [X] T072 Inventory and cut over WS server/commons/Link/WASM/Dart call sites to `KDOM`. FR-051/052.
- [X] T073 Implement `ws_domain.rs` for all WebSocket client/server message kinds and wire `backend/crates/kalamdb-api/src/ws/{messages.rs,events/}`. FR-051.
- [X] T074 Route `link/link-common/src/connection/{websocket.rs,shared/reconnect.rs}`, `link/kalam-link-wasm/src/{helpers.rs,wasm_client.rs}`, and `link/kalam-link-dart/src/api.rs` through `ws_domain`. FR-048/051.
- [X] T075 Reject `serialization=msgpack` and legacy MessagePack frames before effects in `backend/crates/kalamdb-api/src/ws/protocol.rs` and `ws_domain.rs`. FR-052/061.
- [X] T122 Enforce `WS_CONTRACT_DESCRIPTOR_ID` in `backend/crates/kalamdb-serialization/src/ws_domain.rs` and reject stale WebSocket descriptor IDs before effects. FR-061.
- [X] T083 Cache encoded domain row bytes once per `SharedChangePayload` (`domain_row_bytes`) and splice per-subscriber `WsChanges` frames (`encode_change_frame`). FR-060.
- [X] T084 Preserve bounded WS notification channels, batch sizing, and shared connection multiplexing. FR-030/060.
- [X] T086 Use `ReadViewRegistry` and direct `DomainValue` ↔ `ScalarValue` conversions in `kalamdb-commons::conversions` to avoid JSON detours on binary paths. FR-040/050/060.
- [X] T087 Bound `PINNED_DESCRIPTOR_CACHE` (max 1024 `Arc<PinnedRoutineDescriptor>` without retaining JS bodies) and `ReadViewRegistry` (max 4096 handles / 16 MiB). FR-030/054/060.
- [X] T076 Benchmark domain frame encoding/decoding and fanout splicing in `kalamdb-serialization` and `benchv2`. FR-055/062.
- [X] T079 Inventory persistence and Raft codecs and approve the `KDOM` / `KOBJ` boundary per Owner Decision 2 (`scripts/check-serialization-boundary.py`). FR-053.
- [X] T085 Retain packed Raft log/insert/delete/data-response encodings in `backend/crates/kalamdb-serialization/src/protocol.rs`. FR-053/060/062.
- [X] T080 Enforce `KOBJ` / `KDOM` envelope magic and reject legacy unenveloped payloads in `backend/crates/kalamdb-serialization/` and `scripts/check-serialization-boundary.py`. FR-053/061.
- [X] T081 Verify persistence boundary checks via `python3 scripts/check-serialization-boundary.py --fail`. FR-053.
- [X] T031–T037, T039–T041, T042–T044, T047, T049, T088–T091, T093, T059 [P2], T092 [P2], T094–T097, T099–T109, T111–T121 Resolved under Owner Decisions 1–6 in [spec.md](spec.md); remaining actionable code and verification work is consolidated into Phases 1–3 below.

---

## Ready-to-Implement Tasks for `/speckit-implement`

### Phase 1: Binder & Client Binary Invocation Hardening (Gate B2)

- [X] T052 [P] Harden `bind_domain_call_arguments` in `backend/crates/kalamdb-core/src/functions/bind.rs` so `DomainValue::Composite` rejects duplicate slot numbers and unknown slot numbers (`slot == 0` or `slot as usize > input_params.len()`), and `DomainValue::Array` allows omitted nullable parameters (`!param.not_null -> ScalarValue::Null`) matching the `Composite` branch; add unit tests in `backend/crates/kalamdb-core/src/functions/bind.rs`. FR-040/042.
- [X] T057 [P] Support `x-kalam-descriptor-revision` header and explicit error code propagation on `invokeRoutine` in `link/sdks/typescript/client/src/client.ts` and `link/sdks/dart/link/lib/src/invoke_routine_io.dart` (and `cli/templates/schema-gen/typescript/src/generated/kalam.ts.template`), and verify with CLI schema-gen tests in `cli/src/workflow/schema/`. FR-044/061.

### Phase 2: WebSocket MessagePack Cleanup & Callsite Ownership CI (Gate C0 / Gate W)

- [X] T077 [P] Remove dead WebSocket MessagePack caching and helpers (`msgpack_cached`, `to_msgpack`, and `msgpack` feature/test remnants) from `backend/crates/kalamdb-commons/src/websocket.rs`, `backend/crates/kalamdb-commons/Cargo.toml`, and `backend/crates/kalamdb-api/Cargo.toml`. FR-052/056.
- [X] T071 [P] Extend `link/tests/sdk-contracts/check_codec_ownership.py` to scan production WebSocket, API, and SDK files (`backend/crates/kalamdb-api/src/ws`, `backend/crates/kalamdb-commons/src/websocket*`, `link/link-common/src`, `link/kalam-link-wasm/src`, `link/kalam-link-dart/src`) for forbidden `rmp_serde`, `to_msgpack`, or `SerializationType::MessagePack` tokens, and include a built-in negative-control self-test verifying that a synthetic forbidden snippet is rejected. FR-048/050/056.
- [X] T078 Close Gate W after `T077` and `T071` pass `cargo nextest` and `python3 link/tests/sdk-contracts/check_codec_ownership.py`. FR-051/052/055/056.

### Phase 3: UI Overlapping Mutation State, Guarantee Matrix & Final Verification (Gate APP / STABLE / R)

- [X] T098 [P] Fix overlapping same-key pending mutation tracking in `link/sdks/typescript/react/src/hooks/useMutationState.ts` by tracking per-key reference counts (`Map<RowKey, number>`) in `ReducerState` while deriving `Set<RowKey>` for `MutationState`, so completing the first of two concurrent updates/deletes on the same `rowKey` does not clear the pending state early; add a unit test in `link/sdks/typescript/react/tests/`. FR-068; DB-14.
- [X] T110 [P] Publish the per-surface atomicity, isolation, and durability guarantee matrix in `docs/architecture/guarantees.md` and `link/tests/sdk-contracts/guarantee-matrix.json` (covering single SQL statement, HTTP SQL batch, explicit transaction, native function call, PGWire, live subscription, topic consumer, file attachment, and backup/restore per Owner Decision 3), and validate its presence and required surfaces in `link/tests/sdk-contracts/check_codec_ownership.py`. FR-075; DB-02/04.
- [X] T050 Run the complete verification gate across `kalamdb-commons`, `kalamdb-serialization`, `kalamdb-dialect`, `kalamdb-core`, `kalamdb-api`, `link-common`, `kalam-cli`, `python3 link/tests/sdk-contracts/check_codec_ownership.py`, and `python3 scripts/check-serialization-boundary.py --fail`, and mark all tasks complete. FR-001-FR-080.

## Phase 4: Convergence

- [X] T123 CRITICAL Add executable coverage for the `invokeRoutine` descriptor-revision header and typed error code (`KalamRoutineError` in `link/sdks/typescript/client`, `KalamRoutineException` in `link/sdks/dart/link`), document that contract in `docs/sdk/sdk.md`, and record that the sibling KalamSite tree is outside this workspace so it cannot be validated here per Constitution IV (partial)
- [X] T124 Send `application/x-kalam-domain` `KDOM` routine requests from `KalamDBClient.invokeRoutine` and `KalamClient.invokeRoutine` when binary format is selected, and decode the typed response or typed error without the current TypeScript hard failure in `link/sdks/typescript/client/src/client.ts` per US7, FR-039 (partial)
- [X] T125 Run `python3 link/tests/sdk-contracts/check_codec_ownership.py` in `.github/workflows/ci.yml` beside the serialization-boundary step so codec ownership is enforced in CI per FR-056 (partial)
- [X] T126 Replace the stale MessagePack negotiation comments in `backend/crates/kalamdb-api/src/ws/events/mod.rs` and `backend/crates/kalamdb-live/src/notification.rs` with the current domain-frame and JSON behavior per FR-033, FR-052 (contradicts)
