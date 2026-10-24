# Implementation Plan: unified SDK contracts and measured serialization

**Revised:** 2026-10-24. **Status:** Completed. All gates (`A`, `S`, `C0`, `D`, `B2`, `W`, `P`, `APP`, `STABLE`, `R`) and tasks (`T001`..`T121`) are closed.

Read [spec](spec.md), [API](contracts/public-api.md), [protocol](contracts/protocol.md), [performance contract](contracts/serialization-performance.md), [research](research.md) and [tasks](tasks.md) together. The application extension is also normative: [readiness/requirements](app-readiness.md), [application contract](contracts/application-development.md), [database stability](contracts/database-stability.md), [platform research](research-app-platforms.md) and [tasks T094-T121](tasks-app-stability.md).

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

## Existing Slice 1

T001, T019, T018, T038, T045, T046 and T048 remain recorded complete under their original scope. This revision does not claim to rerun their tests. Keep their useful Drizzle, Dart table, Drift and Masky helper implementations, without making their previous wire contracts a requirement for the new cohort.

## Ordered implementation

### 1. Decoder hardening - Gate S

T064-T068 precede public binary exposure. Bound all count, length, offset, recursion, work and decoded-memory use before allocation or native construction. Fix list preallocation, recursive skipping, the ineffective post-decode nesting check and outer-list-length confusion. Include null-heavy values, compressed expansion, malformed offsets and cancellation.

Audit V8 native handle cleanup, ownership, generation and retained memory. Repair or retire unsafe paths. Fuzz the complete decode/access/materialize/encode/cleanup chain and review actual lockfile versions/features. No finite test campaign establishes invulnerability.

### 2. Lean core and measured selection - Gates C0/D

T002-T005/T051 establish metadata, fixtures and baseline. T069-T071 extract the lightweight shared Rust core without Arrow/DataFusion/RocksDB/V8 dependencies in clients. T082 compares the hardened schema-directed Kalam candidate, Postcard and Prost with equivalent dynamic SQL semantics, validation and conversion costs.

Select one production implementation. Competing codecs stay in isolated benchmarks only. Define framing, widths, endianness, presence/null states, exact types, descriptor identity and canonical hashing. Scalar calls should not construct unnecessary Arrow batches; bulk calls should use direct typed builders. Binary processing must not traverse generic JSON trees. Gate D is provisional until integrated Gate R evidence.

### 3. Native routine RPC - Gate B2

T006-T014 and T052-T063 generate current operation descriptors and checked TS/Dart models. Generated methods use the typed executor, not SQL query wrappers. The function route accepts the new-epoch contract. Explicit current typed JSON is an external interface, not a fallback to an older request shape.

Validate descriptors and permissions before effects. Cache immutable plans by revision and scope, not evaluated defaults. Nested calls pass typed values in-process with existing EXECUTE, INVOKER/DEFINER, RLS, transaction, cancellation and deadline semantics. Cross-data-Raft-group atomicity and external/STREAM rollback are not introduced.

T059 advanced views are P2 and excluded from Gate B2 dependencies. T066 handle safety remains P0. Uncertain outcomes do not justify automatic replay or transport switching without authoritative idempotency protection. T110's guarantee matrix and statement-atomicity checks apply to advertised SQL/native behavior; using the same executor is not itself evidence of correct atomicity.

### 4. Shared binary WebSocket - Gate W

T072-T078 apply the selected value codec to distinct WS message kinds: auth/control, subscribe, snapshot boundaries, changes, patches, deletes, errors, cursors and acknowledgments. Keep live ordering, completeness, RLS, replay and scope guarantees.

The target contains no MessagePack WS/server/native/WASM/FRB implementation or transitional adapter. Old negotiation fails explicitly. T083 adds authorization-safe encode-once payload reuse; T084 adds byte/time-bounded batching and backpressure; T086 reduces temporary objects and FFI crossings; T087 bounds pools and retained capacity. JSON exists only as a documented current external representation where needed.

### 5. Fresh persistence and Raft - Gate P

T079-T081 use the selected codec for new KalamDB-owned row/catalog/stream/Raft values with distinct durable kinds. Historical data import is not required. Epoch validation occurs before opening an incompatible store or admitting an incompatible peer. Initialization remains an explicit operator action.

T085 measures lawful WriteBatch/apply/replication batching and repeated-conversion reduction. Keep transaction visibility, WAL, fsync, quorum and commit acknowledgment semantics unchanged. Existing tonic/gRPC may remain an outer transport for canonical payload bytes rather than another domain serializer. RocksDB/Parquet/PGWire/Arrow implementation formats remain ecosystem boundaries.

Test the newly created format through clean/unclean restart, snapshot restore, interrupted writes, corruption and schema revisions. These are current-data correctness tests, not a migration program. T112-T116 extend this to real multi-node fault histories, hot/cold publication and independent backup restore.

### 6. Application completion - Gate APP

T094-T108 and T119-T120 cover real developer setup, secure sessions/organizations, request-scoped SSR, descriptor-driven UI/optimism, keyset/read models, field errors, internal/external operation boundaries, FILE workflows, isolated test helpers, staged deploy and safe diagnostics. Reuse existing owners; do not add a second query engine, cache, auth authority or job orchestrator.

Execute DX-01 school, DX-02 helpdesk/chat, DX-03 clean setup/schema-edit/deploy, and DX-04 multi-identity testkit from application-development.md. Test real TS/browser/SSR and supported Flutter targets. The school races 100 independent clients for one slot, verifying one committed booking and no partial related writes. Fake only external providers, not transaction/permission guarantees.

Target less handwritten infrastructure, not merely shorter examples. Measure equivalent authentication/error/recovery coverage, round trips, runtime resources and at least 50% reduction of generic baseline glue. Preserve business/UI responsibilities. Optional presence/search/sites/browser-persistence/branch merge remain explicitly outside the core; unsupported capabilities cannot be represented as successful fixture results.

### 7. Database stability - Gate STABLE

Begin T109/T110 alongside the foundational work, not after UI completion. DB-01..DB-20 in database-stability.md define precise guarantee/history oracles and faults for atomicity, isolation, SQL correctness, Raft, storage, hot/cold flush, catalog revisions, RLS, live/replay, idempotency, files, SDK lifecycle and restore. Source documentation presently warns direct provider DML does not add statement-wide atomicity; reverify and fix/reroute/reject affected atomic operations rather than papering over the gap.

T111-T118 add concurrency histories with negative controls, deterministic fault hooks plus actual processes, wrong-result differential/metamorphic tests, scoped denial tests, resource pressure, new-format restore and sustained release workloads. Raft is not proof of global SQL serializability. Mocks, source review, checklists and finite fuzzing alone do not certify stability. Unsupported claims must be explicit; unresolved corruption/data-loss/security/false-atomicity defects block release.

### 8. Release evidence - Gate R

T088 tests the fresh-cohort policy and obsolete-path absence; T089 exercises correctness under pressure; T090 runs joint performance gates; T091/T093 reconcile sources and evidence. **T121 APP/STABLE acceptance is also required before Gate R, T093 and final T050 can close.** T092 selective zero-copy stays optional.

Finish table/query/live/Dart durability with untouched TS/Flutter projects and Masky. Generated artifacts come from generators. No source-only assertion closes a runtime task. Publish the capability matrix and tested platform/failure-model scope, not a blanket claim to outperform all platforms or cover every app feature.

## Other feature dependencies

T015-T022 table/named-query work consumes canonical metadata. T023-T030 live semantics precede final WS acceptance. T031-T041 initializes the new local schema and proves same-epoch inbox/projection/action durability. T035 waits for T034 authoritative idempotency. T036 worker fencing precedes concurrent foreground/background claims. T042-T050 complete generation, project examples and package/evidence checks.

T094-T121 are in tasks-app-stability.md, the normative continuation of tasks.md. They extend requirements to FR-080 and criteria to SC-032 through app-readiness.md. T107 depends on actual business atomicity and tested intent boundaries. Conditional capabilities are either implemented and tested or visibly unsupported; new schema evolution and backup obligations do not reintroduce historical-release compatibility.

## Measurement gates

The performance contract specifies equivalent workloads, raw results and proposed targets: 15% lower CPU/op, 20% lower allocated bytes/op, 10% lower retained pipeline memory and 15% greater throughput at equal latency SLO on pre-registered primary workloads. Critical p95/p99 and whole-process memory cannot regress more than 5% beyond noise. These are targets, not measurements. An unmet gate remains open; no hidden scope or durability change is allowed.

Measure native/WASM/FRB, small and large messages, fanout, cold startup, sustained compaction and slow consumers. Report allocation counts and retained capacity separately, plus full RSS, CPU, bytes and tail latency. Upstream project benchmarks supply hypotheses, not transferable speedup factors. APP measures boilerplate/setup/round trips separately from engine throughput.

## Execution discipline

Use current repository instructions, scoped edits and generators. Routine feedback uses repository dev builds and cargo nextest. Benchmark profiles are deliberate, identical and documented. Backend/CLI behavior changes require isolated-server smoke/e2e tests. Documentation-only edits require consistency, link, task-ID and scoped-diff checks; they do not establish runtime test success. No production credentials or live data are test fixtures.

Completion combines S -> C0/D -> B2 -> W -> P, plus APP and STABLE, then R. There is no backward compatibility stage. Optional P2 views never block a safe owned release, while known safety, correctness and durability defects always do.
