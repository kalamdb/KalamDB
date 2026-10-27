# Implementation Plan: unified SDK contracts and measured serialization

**Revised:** 2026-10-24. **Status:** Approved and ready for final task execution. All owner decisions (KDOM codec selection, unified `kalamdb-commons::conversions` scalar authority, WebSocket `KDOM` cutover, TS/Dart `invokeRoutine`, and in-repo verification gates) are resolved in [spec.md](spec.md). Remaining actionable engineering tasks are listed in [tasks.md](tasks.md).

Read [spec](spec.md), [API](contracts/public-api.md), [protocol](contracts/protocol.md), [performance contract](contracts/serialization-performance.md), [research](research.md) and [tasks](tasks.md) together. The application extension ([readiness/requirements](app-readiness.md), [application contract](contracts/application-development.md), [database stability](contracts/database-stability.md), [platform research](research-app-platforms.md) and [tasks-app-stability.md](tasks-app-stability.md)) is folded into the unified execution plan in [tasks.md](tasks.md).

## Current implementation and remaining ready work

| Area | Current state | Remaining ready task |
| --- | --- | --- |
| Shared codec & conversions | `KDOM` (`domain.rs`, `ws_domain.rs`) and unified `DomainValue` ↔ `ScalarValue` conversions in `kalamdb-commons::conversions` are implemented and tested | Approved at Gate D; no competing serializer dependencies |
| Binary & JSON routine HTTP | `functions/invoke.rs` dispatches `KDOM` and typed JSON envelopes to `FunctionService` | `T052`: reject duplicate/unknown composite slots and align array omission in `bind.rs` |
| Generated TS & Dart calls | TS and Dart generators emit `invokeRoutine` with no SQL `CALL`/`SELECT` fallback | `T057`: support binary `KDOM` frame option and deadline/abort handling in TS/Dart `invokeRoutine` |
| WebSocket | Server, native Link, WASM, and Dart bridge use `KDOM` (`ws_domain.rs`) and reject `serialization=msgpack` and stale `WS_CONTRACT_DESCRIPTOR_ID` | `T077` & `T071`: remove dead `msgpack` helpers/features in `kalamdb-commons`/`kalamdb-api` and enforce callsite ownership in `check_codec_ownership.py` |
| UI & Guarantees | React and Flutter live/mutation hooks exist | `T098`: ref-count overlapping same-key mutations in `useMutationState.ts`; `T110`: publish guarantee matrix |
| Verification | Unit, e2e, and `benchv2` suites exist | `T050`: run full verification suite across all affected crates and ownership scripts |

## Release policy

The owner selected a fresh-start release. The new server, nodes, SDKs and generated artifacts use one current format/API epoch. Support for older KalamDB encodings, migration adapters, historical readers, alias layers and a MessagePack transition window is out of scope. Older artifacts produce a clear version error. Bootstrap uses an explicitly chosen empty location; no automatic deletion is allowed. New-format durability, crash recovery and current-schema evolution remain mandatory.

Supported SQL/PGWire, Parquet and explicitly selected Arrow interoperability remain product interfaces. They are not old KalamDB encoding adapters. Historical review notes are background evidence rather than delivery obligations.

## Priorities

P0 is correctness, security, ownership, authorization and durability. P1 is measured reduction in CPU, allocation and retained memory while increasing useful throughput, plus the simplest complete application path. P2 is selective zero-copy. Existing unsafe or leaking native handles require P0 remediation; adding new views is optional. Safe owned values are a valid first implementation.

## Owners

| Area | Owner |
| --- | --- |
| Canonical SQL types and identities | LogicalTypeRef and catalog/commons/dialect contracts |
| Bounded binary values and framing | kalamdb-serialization, optionally with one lean internal core to avoid dependency cycles |
| Arrow/storage conversion | Optional backend adapters, not required by the browser codec |
| Network boundaries | kalamdb-api HTTP/WS plus existing authentication and function/live services |
| Shared client | link-common, native SDK, Rust/WASM and Rust/FRB |
| Generation | CLI schema workflow, target emitters and templates |
| Execution | Existing FunctionService::invoke, binder, ACL, transaction and V8 owners |
| Persistence | Store/tables/streams/publisher/Raft owners; core orchestrates |
| File lifecycle and Parquet | kalamdb-filestore |
| TS tables/UI | Drizzle and React integration |
| Dart remote/local state | kalam_link remote builder; kalam_sync owns one Drift connection |
| App lifecycle/auth/SSR | Existing CLI/auth/session/SDK/framework owners, with request/account-scoped adapters |
| Engine reliability | Existing transaction/store/Raft/flush/catalog tests plus deterministic fault hooks and real processes |

Backend and Link depend on the shared codec; the backend does not depend on an HTTP client for decoding. There is one semantic type authority. Physical adapters do not redefine SQL types. Follow repository rules for source models, dependencies and storage ownership.

## Existing work to retain

T001, T019, T018, T038, T045, T046 and T048 remain historical Slice 1 completions. T002/T051 retain fixture/inventory records. T064/T066/T069 retain narrow source-supported remediation/extraction with attributed prior checks. This revision does not rerun those tests. Keep useful Drizzle, Dart table, Drift, Masky, catalog, KDOM and HTTP work without preserving obsolete wire contracts or assuming unfinished acceptance.

## Ordered implementation

### 1. Decoder hardening - Gate S (acceptance open)

T064-T068 precede public binary release. T064/T066 source repairs are present; do not recreate them. Complete T065/T067/T068 by checking current compression integrity/policy, raw unary function-body buffering, global/tenant/connection allocation/work budgets, endpoint direction allowlists, post-decode conversion charges and resolved dependency advisories. Multipart-upload bounds do not prove the separate function Bytes extractor is bounded before allocation.

Fuzz the complete decode/access/materialize/encode/cleanup chain and test malformed, truncated, oversized, null-heavy, deeply nested, wrong-kind and cancelled requests. Record actual corpus/time/platform/limits. Unit tests are useful evidence but do not stand in for every required campaign. No finite campaign establishes invulnerability.

### 2. Lean core and measured selection - Gates C0/D (acceptance open)

T002-T005/T051 establish semantic metadata, fixtures and baselines. T069's lightweight core is implemented; T070/T071 must prove the actual selected grammar, exact types, native/WASM/FRB use and effective ownership CI. Extend existing check_codec_ownership.py/check-serialization-boundary.py beyond dependency presence and persistence-only roots. Add a negative control that fails when a forbidden WS production encoder is reintroduced.

T082 must compare actual pinned Postcard and Prost libraries with the hardened KDOM candidate, including dynamic SQL mappings, equal framing, validation, resource limits and model-conversion cost. The current local approximation helpers do not satisfy that task. Retain their measurements as preliminary attributed observations only; withdraw their winner/Gate D/R claims. Ship one codec, not a serializer menu. Do not force a new layout before valid results justify it.

### 3. Native routine RPC - Gate B2 (partially implemented, acceptance open)

Preserve the direct KDOM branch in backend/crates/kalamdb-api/src/http/functions/invoke.rs. Finish T052-T057 against actual code:

- Bind recursive logical descriptors, reject duplicate/unknown slots, unify omission/default behavior and verify revision dependency hashes.
- Handle empty/null-first arrays, composite field identity, domains/enums, timestamp-vs-timestamptz, overflow/precision, SQL NULL/JSON null/void and set cardinality without inference from the first value or lossy coercion.
- Define explicit current content negotiation and response/error encoding; distinguish HTTP/auth failures from typed routine errors without silently guessing another format or replaying uncertain mutations.
- Add the mandatory typed executor to real native Link, WASM/TS and FRB/Dart clients. A re-export or optional generated callback is not an implementation.
- Remove SQL fallback and old namespace/KalamFunctions aliases from source templates; emit checked descriptors/codecs and separate function-only imports from sync/ORM. Regenerate, do not hand-patch outputs.

Trace generated method -> actual client -> bridge -> wire -> binder -> FunctionService -> typed response. Real-server tests must prove zero SQL-parser entry for generated routine calls. Keep intentional raw SQL and PGWire available.

Nested calls preserve EXECUTE, INVOKER/DEFINER, RLS, root transaction/deadline/cancellation and native typed values. T059 views remain P2; T066 safety stays P0. Cross-data-group atomicity and external/STREAM rollback are not introduced. T110's statement-atomicity/guarantee matrix remains binding. Close B2 only with T060-T063 and prerequisites actually evidenced.

### 4. Shared binary WebSocket - Gate W (incomplete)

Start from backend/crates/kalamdb-api/src/ws/{messages.rs,events/}, kalamdb-commons/src/websocket_protocol.rs, link/link-common/src/connection/websocket.rs, link/kalam-link-wasm/src/helpers.rs and actual Dart/FRB callers. T072 inventories all send/receive/control paths; T073/T074 connect them to the selected domain codec. Include auth, subscribe/unsubscribe, heartbeat, snapshot boundaries, changes/patches/deletes, errors, cursors/acks and reconnect.

T075/T077 directly reject and remove MessagePack negotiation/defaults/enum branches/encoders/decoders/dependencies; there is no compatibility window. Use real server/browser/native/Flutter tests. Defined Ws* kinds alone do not satisfy this gate.

T023/T024 establish actual delivery cursors, not lastVersionId checkpoints. T083/T084 add authorization-safe encode-once fanout and bounded batching/backpressure. T086 removes generic JSON-string bridges and per-cell ScalarValue churn only after measured alternatives; genuine JSON/JSONB data semantics remain. T087 accounts for retained capacity and compact copying. Gate W waits for live semantic and platform evidence, not only codec-unit roundtrips.

### 5. Fresh persistence and Raft - Gate P (incomplete)

T079-T081 inventory the actual generic/object/model/row/stream/Raft callers and replace owned domain values using the selected core. Current object/generic.rs FlexBuffers and protocol.rs StateMachine MessagePack are concrete unfinished integration. Distinct durable kind/version envelopes remain legitimate, not competing value serializers.

No historical import, dual reader or rollback-to-old-format layer is required. Old epochs fail explicitly without automatic deletion. Keep external RocksDB/Parquet/PGWire/Arrow containers and tonic outer transport narrowly scoped. T085 measures lawful WriteBatch/apply/replication batching with unchanged WAL/fsync/quorum/visibility and commit acknowledgments.

Test current-format clean/unclean restart, snapshot restore, interrupted writes, corruption and schema revisions. T112-T116 extend this to multi-node histories, hot/cold publication and independent restore. Starting fresh does not waive future durability.

### 6. Application completion - Gate APP (acceptance evidence open)

T094-T108/T119-T120 remain open until pinned implementation and execution evidence covers the existing app contract. Reuse CLI/auth/session/SSR/Drift/UI/jobs/files owners. Do not infer implementation absence from missing evidence or create a second runtime/cache.

A specific unfinished case is T098: React pending update/delete Sets still clear by key rather than by invocation. Prove same-key overlaps, optimistic rollback and account/lifecycle fencing. Native generated routines and delivery cursors must work before end-to-end apps can satisfy their target contract.

Execute DX-01 school, DX-02 helpdesk/chat, DX-03 setup/schema-edit/deploy and DX-04 multi-identity helpers with real supported clients. Race 100 independent one-slot bookings with actual constraint/transaction enforcement. Fake external providers only, not database guarantees. Compare handwritten generic glue with equivalent auth/error/recovery scope; no superiority claim from shortened examples.

### 7. Database stability - Gate STABLE (acceptance evidence open)

Start T109/T110 early. DB-01..DB-20 cover SQL wrong-result oracles, statement atomicity/isolation, Raft/storage faults, hot/cold publication, catalog revisions, RLS, replay, idempotency, files, SDK lifecycle and backup/restore. Reverify the architecture's direct-DML statement-atomicity caveat and fix/reroute/reject affected operations before advertising atomic SQL.

T111-T118 must provide the actual model, seeds, real-process fault histories, negative controls, restore drill, supported platform coverage and recorded sustained workload. Raft does not prove global serializability. A checked task or mock is not a substitute for its oracle. Unresolved wrong results, data loss, scope leakage, false atomicity or corrupt restoration block release.

### 8. Release evidence - Gate R (open)

T088 verifies actual clean-break behavior and obsolete-path absence; T089 verifies integrated correctness under pressure. T090 requires raw pre-registered CPU/memory/throughput/tail-latency results. T091/T093 reconcile real source and advisory evidence, with preliminary reports labeled honestly. T121 APP/STABLE evidence precedes Gate R and final T050. T092 remains optional P2.

Do not mass-close tasks. For each task attach implementation commit, production callers, test command, platform/features, result artifact, negative controls, missing/skipped scope and dependent-gate status. Authoring a test is not executing it. A successful helper test does not show that production calls it.

## Other feature dependencies

T015-T022 consume canonical metadata. T023-T030 live semantics precede WS acceptance. T031-T041 must prove current-epoch inbox/projection/action durability using one Drift connection. T035 waits for T034 authoritative idempotency; T036 fencing precedes concurrent workers. T042-T050 require actual generated examples and publication/package evidence. Keep all requirements even where current code falls short.

## Measurement gates

The existing performance contract proposes 15% lower CPU/op, 20% fewer allocated bytes/op, 10% lower retained pipeline memory and 15% more throughput at equal latency SLO on pre-registered workloads. Critical p95/p99 and whole-process memory must not regress more than 5% beyond noise. These are targets, not observed results. Unmet gates remain open without hidden changes to weights, validation, precision, RLS or durability.

Measure real native/WASM/FRB, tiny and large payloads, fanout, low load, cold startup, slow readers and sustained compaction. Distinguish live allocations, retained capacity and full RSS. Small safe copies may outperform large retained views; no optional zero-copy work blocks a safe owned implementation.

## Execution discipline and evidence

Use current repository instructions and scoped edits. Default dev builds/cargo nextest apply; benchmark profiles are deliberate, equivalent and documented. Backend/CLI behavior changes need isolated-server smoke/e2e tests. This reconciliation is documentation-only: inspect diff/IDs/links and verify remote commit/branch, but do not claim runtime tests, benchmarks or production changes.

Before final acceptance trace both generated TS/Dart RPC and server-to-native/browser/Flutter live data through the same selected codec, verify old paths removed and connect each claim to evidence. Read [implementation audit](implementation-audit-2026-10-24.md) first to avoid redoing repairs already present.
