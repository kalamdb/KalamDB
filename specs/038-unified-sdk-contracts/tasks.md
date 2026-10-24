# Tasks: unified SDK contracts, fresh epoch and measured codec

**Revised:** 2026-10-24. **Status:** In progress after [source/evidence audit](implementation-audit-2026-10-24.md) of `81e5fb15dc600d268581c1b22ea25b74e25b77f1`. Blanket completion is withdrawn. IDs and acceptance scope are retained. See [spec](spec.md), [plan](plan.md), [protocol](contracts/protocol.md), [performance contract](contracts/serialization-performance.md) and normative [T094-T121 continuation](tasks-app-stability.md).

An open checkbox means **acceptance is not established**, not necessarily that implementation is absent. The audit distinguishes contradicted/partial code, pending evidence and optional work. Reuse working code and existing tests. Retained checked IDs are T001/T002/T018/T019/T038/T045/T046/T048/T051/T064/T066/T069: historical or narrow source-supported records, not an independently rerun suite. T059/T092 are optional P2. Every reclosure needs task-specific implementation and execution evidence; no mass checkbox closure.

## Recorded Slice 1 completion

These seven markers remain historical records in their original scope. They do not require preserving the old API/wire format or certify the current cohort.

- [X] T001 [S1] Record the SDK surface and Masky generic-glue baseline; retain unrelated working-tree edits.
- [X] T019 [S1] Emit the TS Drizzle kTable schema and driver/live/file/conflict-update behavior without a second table API.
- [X] T018 [S1] Add the thin Dart remote table builder, qualified tableId/liveSql/sync metadata and full-row/partial-patch decoding.
- [X] T038 [S1] Add managed, app-constructed and attached single-Drift modes, session reconciliation, auth recovery, debounced state, retryable initialization and checkpoint/projection helpers.
- [X] T045 [S1] Adopt those helpers in Masky while retaining domain/REST/UI responsibilities and one local database connection.
- [X] T046 [S1] Record the affected TS/Dart package regression validation for the original Slice 1.
- [X] T048 [S1] Document those Slice 1 APIs and sync architecture.

## Baseline and canonical semantics - Gate A

- [X] T002 Establish shared schema/type/routine/live/adversarial fixtures and native/TS/Dart harness entry points under `link/tests/sdk-contracts`. Capture pre-change measurements and workload manifest before choosing a codec. FR-032/062. Retained as fixture/baseline record, not end-to-end release evidence.
- [ ] T003 Extend catalog/contract metadata for real ordered keys, defaults/identity/generated fields, recursive types, routine modes/overloads/cardinality and relevant entity revisions. FR-002/004/010/011. Existing compiler work is credited; attach complete semantic acceptance evidence.
- [ ] T004 Implement deterministic names, exact SQL/public-name mapping, scoped hashes, current-epoch manifest and collision tests. FR-002/007. Verify existing implementation rather than recreating it.
- [ ] T005 Define exact checked scalar/value/error mappings: wide integers/decimals, precision-preserving temporal values, bytes/files, SQL NULL/JSON null/missing and nested errors. Replace lossy accessors in new APIs. FR-005/006/040. Audit A04 identifies unresolved empty-array, slot, temporal and null/cardinality paths.
- [X] T051 Inventory PR #402 execution/binder/serialization/Link/WASM/FRB paths and Spec 037 dependencies; separate inspected code from runtime evidence. Include existing tonic/Raft and external format boundaries. FR-047/048. Retain the original inventory with the dated audit as its current supplement.

Gate A: one semantic owner and fixtures; neither emitter invents key, type or nullability rules. Whole-gate evidence remains open.

## Stage 1: harden before exposure - Gate S

- [X] T064 [P0] Fix untrusted preallocation in `row/value.rs`, bound recursive `skip_value`, validate embedding dimensions and checked offset/count arithmetic. Replace `row/batch.rs::assert_nesting` with parsing-time depth/work/element/allocation budgets; outer ListArray length is not its child element count. Cover null-heavy and zero-sized logical elements. FR-049/058. Retain the inspected remediation and attributed narrow tests; broader budget/gate acceptance remains below.
- [ ] T065 [P0] Replace unbounded or permissive decompression helpers across server/native/WASM/FRB; finite compressed/output/work/window limits, explicit format selection, no failed-decode fallback. Ordinary small calls start uncompressed. FR-057. Existing bounded helpers are progress; verify actual callers, integrity and selected compression policy.
- [X] T066 [P0] Audit existing native V8 read handles and buffer allocators for ownership, cleanup, generation, aliasing, projection, cancellation and actual pinned bytes. Fix or disable unsafe/leaking paths. Do not depend on optional new views. FR-046/054/058. Retain the bounded generation-fenced registry remediation and attributed test, not an expanded zero-copy/security certification.
- [ ] T067 [P0] Add fuzz/property/negative tests for frame -> decode -> validate -> field access -> Arrow/V8/SDK conversion -> encode -> cleanup. Exercise arithmetic, truncation, nested/skipped values, type mismatch, compression, concurrent cancellation and per-tenant memory limits. FR-032/049/058. Record real execution/corpus/time/platform; named unit fixtures alone do not complete this task.
- [ ] T068 [P0] Gate S: record bounded streaming bodies independent of Content-Length, preallocation/depth/work tests, actual lockfile/advisory review, suitable sanitizer/Miri or equivalent evidence and no known unresolved decoder/handle safety defect. Depends T064-T067. FR-049/057/058. Verify unary function Bytes extraction before buffering, not only multipart upload helpers; cover conversion and aggregate budgets.

## Stage 2: lean core and measured decision - Gates C0/D

- [X] T069 Extract/refactor one `kalamdb-serialization`-owned lean core callable by backend and Link/native/WASM/FRB without mandatory Arrow/DataFusion/RocksDB/V8/server dependencies. Optional physical adapters derive from canonical logical types; no circular dependency or duplicated type authority. FR-048/050. Retain optional backend-adapter extraction and attributed lean build evidence; this does not prove production callers use it.
- [ ] T082 Pre-register workload/SLO/metric matrix, then compare hardened Kalam schema-directed encoding, Postcard and Prost with identical dynamic SQL contracts, validation and materialization. Pin versions and include failed cases, CPU/memory/latency and WASM footprint. Select one provisional production winner at Gate D; no codec menu. Depends T002-T005/T068/T069. FR-055/059/062. Replace handwritten Postcard/Prost-style proxies with actual libraries and equivalent framing/budgets; withdraw the preliminary winner claim until valid evidence exists.
- [ ] T070 Implement the selected common value/framing codec with explicit widths/endian, presence/null semantics, descriptor identity, bounded grammar, endpoint message-kind separation, canonical digest policy and byte-level golden vectors. Do not freeze KOBJ/protobuf/custom byte layout merely because it was proposed earlier. Depends T082. FR-048/051/059. KDOM exists and is the current candidate; complete descriptor/direction/format tests and integrate actual callers.
- [ ] T071 Gate C0: native/WASM/FRB conformance, dependency-size check and repository-wide codec-ownership CI. Cover API/commons/Link/WASM/FRB; narrowly list current external JSON/PGWire/Parquet/Arrow/tonic transport exceptions. No old-reader exemptions. Depends T070/T068. FR-048/050/056. Existing checks only prove dependency presence/lean tree; add production-callsite scans and failing negative controls.

## Generated current API and server contracts

- [ ] T006 Generate recursive model validators and typed bridge bindings from the common semantic tree; use shared Rust binary codecs, not hand-written target-language binary parsers. FR-004/005/006.
- [ ] T007 Emit models/codecs/tables/operations/functions/queries/facade and optional adapters with one current API epoch and staged multi-root publication. Failed generation preserves working files; no previous-API emitter/alias layer. FR-002/003/029.
- [ ] T008 Generate immutable operation/bound-call descriptors with stable identity, modes/defaults, result/cardinality and codec/ABI revisions. FR-004/026. The current minimal TS descriptor is not the complete acceptance contract.
- [ ] T009 Add paired typed get/getOne/getOptional/execute/invoke APIs and checked errors; raw SQL is a current product interface, not generated routine transport. FR-006/008/038. Implement the executor on actual clients/bridges rather than only an optional template interface.
- [ ] T010 Generate createKalamFunctions and current functions/operations facades; remove obsolete namespace and KalamFunctions aliases rather than adapting them. FR-001/026/029. Separate function-only imports from ORM/sync dependencies.
- [ ] T011 Compile positive/negative generated cases for nested types, arrays, domains, keys, omission/null and scalar/set/void results. FR-004-FR-007/032.
- [ ] T012 Add authorized current-epoch capability/contract endpoints with scoped ETags/cache and no private bodies/secrets. Unknown epochs fail, never downgrade. FR-026/028/031/061. Reuse existing endpoint work; attach whole-task evidence.
- [ ] T013 Bind the same catalog routine across SQL and new HTTP/native entry points; test ambiguity, authorization, forbidden execution metadata and result validation. FR-026/031/043.
- [ ] T014 Generate accurate current OpenAPI paths, nested/scalar schemas, defaults/nullability/security and errors. Keep optional exports only if product-supported; no old exporter compatibility requirement. FR-027.

## Stage 3: native typed RPC - Gate B2

- [ ] T052 Resolve/cache immutable revision-pinned descriptors from authoritative metadata, with type/layout/settings/security dependencies. Defaults are evaluated at invocation, not cached as values. Depends T003-T005/T008/T012. FR-042. Verify unambiguous digest framing, transitive type changes, duplicate/unknown slots and omission parity in existing bind.rs.
- [ ] T053 Add the new-epoch native function HTTP envelope and optional explicitly selected typed JSON representation to the existing route/service. No old request-shape decoder and no generated SQL. Public binary exposure waits for S/C0. FR-038/039/043/044. Preserve existing KDOM dispatch; finish explicit content/error/revision policy instead of undocumented sniff/fallback behavior.
- [ ] T054 Change generated TS/Dart routines to typed executor calls; remove SQL wrapper transport and obsolete aliases. Regenerate fixtures and compile real clients. Depends T006-T010/T053. FR-038/040. TS optional invokeRoutine plus client.query fallback and Dart _client.query are concrete blockers.
- [ ] T055 Run exact numeric/temporal/nullable/nested/default/cardinality and wrong-type conformance in TS/native/WASM/Dart/FRB and SQL. Include unsupported cases and current schema changes. FR-040/032. Explicitly exercise empty/null-first arrays, composite names/slots, temporal range/precision and void/null/set distinctions.
- [ ] T056 Integrate selected binary codec request/results directly with routine values and FunctionService; no JSON or MessagePack intermediate and no competing RPC serializer. Depends T070/T071/T052/T055. FR-039/040/048. Existing HTTP path is partial progress; conversions must use authoritative recursive descriptors.
- [ ] T057 Implement native/WASM/FRB direct invocation, current-epoch capability checking, finite deadlines and safe unknown-outcome errors. No transport downgrade or uncertain mutation retry. Depends T056/T012. FR-044/061. Trace the actual generated-client-to-server path on each target.
- [ ] T058 Pass typed owned values in nested in-process calls; remove avoidable JSON envelopes and repeated conversions while enforcing separate permission frames, same root transaction/deadline/cancellation and checked results. Depends T052/T055/T066. FR-041/043.
- [ ] T060 Gate B2 behavioral evidence: SQL/typed JSON/binary/nested parity for ACL, RLS, INVOKER/DEFINER, signature changes, malformed frames, timeout, rollback/topic staging, unsupported cross-group writes and external/STREAM effects. Depends T053-T058 and S/C0, NOT T059. FR-040-FR-044. Real SDKs and parser-entry assertions required, not mocked native invocation.
- [ ] T061 Record RPC CPU/allocations/retained memory, p50/p95/p99, fixed-SLO throughput, bytes and native/WASM/FRB costs against valid baselines. Evidence must include realistic handlers, not only echo. FR-045/055/062.
- [ ] T062 Align API/protocol/OpenAPI/quickstart and runtime docs to actual completed current-epoch behavior; no old version adapters or unmeasured zero-copy claims. FR-033/047.
- [ ] T063 Sign off RPC capability only after T060-T062 evidence; codec choice remains subject to final Gate R integration. Optional P2 work does not block. FR-038-FR-047.

## Typed table and query track

- [ ] T015 Define immutable typed query plans, predicates, cardinality, cancellation and feature checking. FR-008/010/011. Existing metadata work is credited; whole-task evidence is pending.
- [ ] T016 Analyze named SQL queries with canonical AST/planner to generate input/result/key/live metadata. FR-010/011.
- [ ] T017 Emit catalog-derived row/key/insert/patch/column models; reject guessed keys and forbidden writes. FR-004/007/008.
- [ ] T020 Generate current-epoch Drift mirror tables for the one session database; no second remote/local ORM or duplicated domain models. FR-009/019/035.
- [ ] T021 Run paired real-server CRUD/default/key/quoting and Drizzle/Drift/model agreement tests. FR-004/005/008/011/032.
- [ ] T022 Remove redundant quote/default/file SQL rewrites only when authoritative binding behavior is proved; keep current typed upload semantics. FR-011.

## Live semantics track

- [ ] T023 Implement snapshot begin/end, delivery identity, scoped history/domain cursors, typed parameters and explicit reset/gap outcomes for the current protocol epoch. FR-012/013/016.
- [ ] T024 Carry metadata and explicit acknowledgment through native/WASM/FRB; no row-max replay guesses or hand-patched generated bridges. FR-016. Current lastVersionId callback checkpoints are not proof of delivery-cursor acceptance.
- [ ] T025 Extend the existing TS live controller and pair it with Dart handles, correct seeded state, cancellation generations and disposal. No duplicate state machine. FR-012/014.
- [ ] T026 Implement scoped subscription leases/deduplication and deterministic teardown with account/auth/projection tests. FR-014/023.
- [ ] T027 Reconcile full/partial/key-only-delete/filter transitions, ordering and local windows under hard resource limits. FR-013/015/030.
- [ ] T028 Use structured generated descriptors and canonical raw SQL parsing; remove regex-based alternate query interpretation. FR-011/015.
- [ ] T029 Bind current React hooks and Flutter examples to the same live snapshots/lifecycle. FR-001/012.
- [ ] T030 Prove paired live traces including empty/multi-batch snapshots, top-N promotion, replay gaps, wrong domains/scopes, observer sharing and failures. FR-012-FR-016/032.

## Stage 4: one binary WebSocket and direct MessagePack removal - Gate W

- [ ] T072 Inventory WS/server/commons/Link/native/WASM/FRB encode/decode/negotiation/dependency call sites and prepare one coherent current-epoch cutover. No transitional stack. FR-051/052. Use audit A02 as the starting inventory, including both sending and receiving.
- [ ] T073 Implement the shared codec's bounded WS kinds for auth/control, subscribe, snapshot boundaries, changes/patches/deletes, ack/cursor/error/reset, preserving RLS, ordering and replay. Depends B2/T023-T027/T070. FR-051. Actual messages.rs/events callers must replace MessagePack; Ws* enums alone do not satisfy this task.
- [ ] T074 Route native Link/TS-WASM/Dart-FRB WS parsing and typed materialization through the common Rust codec. No generic JSON/MessagePack trees in the binary path. FR-048/051. Update websocket.rs, WASM helpers and real bridge clients, regenerate outputs and test socket traffic.
- [ ] T075 Test current-epoch handshake/capability agreement and explicit rejection of old MessagePack, old peers, unknown kinds and stale descriptors before effects; no downgrade/compatibility adapter. FR-052/061.
- [ ] T083 Encode live payload once per identical authorized projection/type/codec/compression group, with per-subscriber framing, bounded cache and no cross-tenant or stale-permission reuse. Depends T073/T074. FR-060.
- [ ] T084 Add byte/count/time-bounded WS microbatching and byte-budgeted queues, reuse authenticated connections and implement fair pressure/disconnect-resume policy. Preserve intermediate events and transaction semantics. FR-030/060.
- [ ] T086 Remove measured ScalarValue/one-element-array churn in bulk Arrow decode and JSON bridge detours; compare batched typed-array/model conversion and FFI crossing cost on real browser/Flutter runtimes. Scalar calls stay scalar. FR-040/050/060. Current Vec<Vec<ScalarValue>> and JSON callback paths remain concrete optimization work.
- [ ] T087 Bound pool/cache capacities and idle retention, compact-copy oversized backing slices when beneficial, and expose retained-capacity metrics. No automatic unsafe borrowing requirement. FR-030/054/060.
- [ ] T076 Benchmark WS current baseline versus selected binary, including small patches, large snapshots, authorized fanout, slow consumers, reconnect/ack and constrained clients. Apply performance contract, not unchecked access timings. FR-055/062.
- [ ] T077 Remove all production WS MessagePack enum/parameter/codec branches, defaults, dependencies and obsolete tests/docs in server/native/WASM/FRB. Keep an old benchmark executable isolated, never a production fallback. FR-052/056. Effective production-callsite CI and a failing negative control are required.
- [ ] T078 Gate W: all paired live traces, pressure/scope/replay tests, ownership CI and old-format rejection pass, with actual performance evidence. Depends T072-T077/T030/T083/T084/T086/T087. FR-051/052/055/056. Gate is contradicted at the audited baseline and cannot close while production MessagePack remains.

## Stage 5: fresh persistence and Raft - Gate P

- [ ] T079 Inventory all KalamDB-owned catalog/row/stream/raft/snapshot serialization and clearly distinguish external RocksDB/Parquet/PGWire/Arrow/tonic transport formats. Specify current epoch/domain kinds; do not plan historical readers. FR-053.
- [ ] T085 Profile existing lawful WriteBatch/Raft apply/replication paths; reduce redundant domain encoding and benchmark bounded grouping without changing WAL/fsync/quorum, visibility or acknowledgments. FR-053/060/062.
- [ ] T080 Implement the selected codec for new owned durable/Raft values and remove old owned readers/writers. New empty stores initialize explicitly; old format/cluster epochs fail without automatic deletion. Test new-format crash/restart, snapshot, corruption and schema revisions. FR-053/061. object/generic.rs FlexBuffers and protocol.rs StateMachine MessagePack still require integration work.
- [ ] T081 Gate P: equivalent-durability persistence evidence, new-format recovery, owned-codec consolidation and dependency cleanup complete. No old data migration, rolling mixed-version or read-old/write-new task remains. FR-053.

## Dart durability track

- [ ] T031 Initialize a fresh current-epoch Drift schema for query generations/membership, inbox/receipts, actions/results/overlays and fenced workers. No older-store migration. New accepted work must survive restart. FR-017-FR-023/035.
- [ ] T032 Implement lazy cached-first mirror with independent query membership, atomic generation activation, empty resnapshot and bounded local observation. FR-017/025/030.
- [ ] T033 Implement consumers, typed outcomes, same-database transactional projections and explicitly receipted foreign projections. Network effects remain in actions outside local transactions. FR-018/019/025/035.
- [ ] T034 Implement authoritative idempotency: atomic database effects/receipt, canonical digest, identity conflicts, expiry/replay, uncertain external effects and cancellation. Defaults are evaluated once per accepted invocation, not on replay. FR-020/021/031.
- [ ] T035 Generate only eligible durable operation handles/results/restoration; unsafe calls cannot enter automatic retries. Depends T034. FR-020/021.
- [ ] T036 Add worker fencing, unresolved per-key ordering, dependencies, optimistic overlays and accepted/reconciled distinction. FR-022/023.
- [ ] T037 Reuse current typed operation codecs for custom Dart REST actions with explicit downstream idempotency/retry policy. FR-003/020.
- [ ] T039 Implement total/per-consumer catch-up budgets, completion/hasMore/errors and safe cancellation/drain on the shared connection. FR-024.
- [ ] T040 Replace unbounded history scans/watchers with indexed bounded claims/aggregates/retention and typed scoped diagnostics. FR-025/030.
- [ ] T041 Run same-epoch crash tests across receipt/apply/ack/execute, foreign projection replay, account changes, competing workers, expired receipts and optimistic ordering. FR-017-FR-025/032/035.

## Project, release and evidence - Gate R

- [ ] T042 Set one current generator API epoch, `schema gen --check`, atomic owned-output publication and cleanup; no old output aliases. FR-002/028/029.
- [ ] T043 Update TS/Dart templates and dev/watch readiness for the coordinated cohort, checked server handlers and actionable failures. FR-026/028.
- [ ] T044 Turn current API/quickstart examples into real compiling/running fixtures; include untouched projects and same-epoch schema changes. FR-001/028/032. Model typechecks do not prove actual binary requests or real app behavior.
- [ ] T047 Run 1k/10k/100k sync/live/catch-up/retention workloads and measure Masky generic-glue reduction, not domain/UI deletion. FR-030/037/062.
- [ ] T049 Align server/TS/Dart/generator epoch and package cohort, eliminate unused compatibility code and verify dependency ownership. FR-003/028/029/033.
- [ ] T088 Test the clean-break contract: old SDK/protocol/store/local-cache/cluster epochs are rejected explicitly, no runtime adapters exist and no directory is erased automatically. Scan all active docs/tasks for contradictory preservation requirements. FR-029/061.
- [ ] T089 Run integrated permission/type/transaction/live/durable correctness under high concurrency, slow consumers, reconnect, cancellation and allocation pressure. Equivalent semantics are prerequisite to speed claims. FR-031/032/058.
- [ ] T090 Gate R measurements: pre-registered CPU/allocated/retained memory, fixed-SLO throughput, latency and client-footprint targets from serialization-performance.md. Include cold/warm and sustained storage conditions, raw repetitions and uncertainty. Unmet targets keep the gate open. FR-055/062.
- [ ] T091 Recheck primary-source references and advisory applicability, record actual toolchain/lockfiles, and separate source observations/proposed targets from measured outcomes. FR-033/047/058.
- [ ] T093 Review all public speed/security claims against evidence; publish selected codec rationale and limitations, no blanket zero-copy/invulnerability claims or silent target relaxation. FR-059/062. Correct preliminary comparison/blanket closure conclusions before release.
- [ ] T050 Final cross-artifact/current-API/security acceptance: all mandatory requirements and SC-001-SC-032 have evidence; absent P2 work is explicitly deferred, not advertised. Depends mandatory gates, T093 and T121 APP/STABLE. FR-001-FR-080. A documentation-only correction cannot close this task.

## Optional P2 only - never a prerequisite of S/C0/D/B2/W/P/R

- [ ] T059 [P2] Explore validated native read views or exclusive-buffer transfer only after a safe owned implementation and profiles demonstrate a material bottleneck. Require ownership/projection/generation/cancellation/pin-budget tests and a benefit over compact copying. FR-046/054. Deferred pending measured justification, not declared complete.
- [ ] T092 [P2] Measure any proposed zero-copy extension end-to-end with full validation and identical field reads. Reject it if retained memory, security complexity or tail latency worsens. Existing unsafe lifetime bugs remain covered by mandatory T066. FR-046/054.

## Dependencies and completion rules

Mandatory sequence: A and S -> lean core plus D selection -> C0 -> B2 -> W -> P -> R, with APP/STABLE and T121 required before final release. T082 can use isolated prototypes, but public binary acceptance waits for S/C0. WS acceptance waits for live semantics, and automatic durable actions wait for server receipts. Fresh persistence planning can begin earlier; final acceptance retains unchanged durability. New optimized batching cannot alter user-function invocation counts, volatile behavior or transaction scope.

T001-T093 appear exactly once as task definitions here. Twelve records remain checked; 81 are open in this file, plus 28 open in tasks-app-stability.md. The audit ledger explains why. Open means contradicted/partial, evidence pending or optional deferred; it does not instruct a blind rewrite. Every reclosure names implementation SHA, production paths, actual tests, platform/features, raw results, negative controls, skipped limits and prerequisites. Use repository instructions/dev builds/cargo nextest, isolated test data and focused validation. This documentation correction runs no runtime tests.

| Requirements | Main work | Acceptance |
| --- | --- | --- |
| FR-001-FR-011 | T002-T022, T042-T044 | SC-001/002/004/006/007 |
| FR-012-FR-019 | T023-T033, T073-T078 | SC-003/005/017 |
| FR-020-FR-025 | T031-T041 | SC-005/013 |
| FR-026-FR-037 | T006-T014, T042-T050, T088 | SC-004/009/010 |
| FR-038-FR-047 | T051-T063, T066 | SC-011-SC-014 |
| FR-048-FR-058 | T064-T081, T083-T089 | SC-015-SC-020 |
| FR-059-FR-062 | T082-T093 | SC-019/021/022 |
| FR-063-FR-080 | Normative tasks-app-stability.md T094-T121 | SC-023-SC-032, DX-01..04, DB-01..20 |
