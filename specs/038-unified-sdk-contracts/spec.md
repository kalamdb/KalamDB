# Feature Specification: Unified TypeScript and Dart SDK Contracts

**Feature directory:** `specs/038-unified-sdk-contracts`  
**Created:** 2026-10-18  
**Revised:** 2026-10-22 - research-backed fresh-start release, application readiness and database stability  
**Branch:** `feat/pg-dialect`, PR #402  
**Status:** Implementation specification. New runtime behavior, stability and performance are NOT established by this document.

## Decision and precedence

This revision replaces earlier preservation, dual-version, migration-window, alias and old-reader requirements in this feature. The owner explicitly authorized a fresh start: **no historical-data preservation and no backward compatibility** for old KalamDB-owned storage, Raft payloads, WebSocket messages, SDK contracts, generated files or local sync formats. There is no temporary MessagePack adapter or staged read-old/write-new rollout. Ship server, nodes, SDKs and generated artifacts as one new format/API cohort.

Reject old files, peers and SDK contracts before effects with an actionable format/epoch error. Do not silently reinterpret or automatically delete user directories. Clean bootstrap uses explicitly selected empty directories; this specification edit does not authorize deleting any deployed data. Once bootstrapped, durability, crash recovery, replay and preservation of newly accepted effects remain mandatory.

Version fields, current-schema revisions and same-epoch DDL remain necessary for correctness; they do not promise support for an older release. SQL/PGWire, Parquet and explicitly supported Arrow interoperability are product interfaces, not obsolete KalamDB adapters. Spec 037 remains authoritative for SQL meaning and security. For this delivery, 038 supersedes any old requirement to block release on zero-copy or to preserve older KalamDB encodings. Historical review notes are diagnostic evidence, not current release policy.

The [application readiness extension](app-readiness.md) is normative: US9/US10, FR-063..FR-080, SC-023..SC-032 and [T094..T121](tasks-app-stability.md) extend this specification. [Application development](contracts/application-development.md) and [database stability](contracts/database-stability.md) add APP and STABLE prerequisites to Gate R and final T050. They do not create another SDK/serializer/transaction architecture or restore old-release compatibility.

## Goal and priorities

Generate one checked application contract from the SQL catalog for TS and Dart, and use **one Rust-owned binary domain-value codec** through backend, `link-common`, WASM and FRB. Native routine calls must not synthesize SQL. WebSocket and owned persistence/Raft payloads use the selected codec with distinct message kinds. JSON is an explicit external representation, never a mandatory intermediate tree for binary calls.

Priority order: **P0 correctness/security/durability; P1 lower CPU, allocations and retained memory with higher measured throughput and less application boilerplate; P2 selective zero-copy.** Safe owned values and deliberate copies are acceptable. Fix existing native handle leaks/lifetime errors at P0; do not make new unsafe views a prerequisite.

The architecture is selected, but no library is certified fastest by external benchmarks. Gate D compares the hardened Kalam schema-directed candidate against Postcard and Prost using identical SQL semantics and validation. Only one wins production use. Alternatives remain isolated benchmarks, not negotiated production serializers. [Research](research.md) explains the sources; [performance contract](contracts/serialization-performance.md) defines measurable selection and release gates.

## User stories and acceptance

### US1 - Generate and call the same application API (P1)

Generate types, keys, inserts, patches, operation references and codecs once. Compile TS and Dart positive/negative fixtures. Nested types, arrays, domains, enums, defaults, overloads and result cardinality agree. Invalid remote values fail decoding instead of passing through assertions. Function-only imports avoid ORM/sync/UI dependencies. Failed generation leaves the previously valid working output intact; that publication safety is not support for an older API.

### US2 - Typed table access without repeated boilerplate (P1)

TS uses Drizzle and generated `kTable` schemas; Dart uses a thin remote builder in `kalam_link`. Real keys come from the catalog, not a field called `id`. Writable/default/generated fields are distinguished. Missing patch fields differ from null. Named queries are analyzed through the canonical SQL parser and retain quoting, types and cardinality. Local Drift and remote query execution remain distinct.

### US3 - Typed live queries (P1)

The same descriptor supports get and live operations. Paired tests prove empty/multi-batch snapshots, explicit completion, insert/patch/delete/filter transitions, ordering, shared observers and bounded retained state. Cursor/replay gaps are explicit; socket connectivity is not snapshot completion. No client silently loses changes during reconfiguration, reconnect or pressure.

### US4 - Dart persistence and recoverable actions (P2)

Use one managed or attached Drift connection per account. Fresh local stores begin in the new epoch; no old-store import is required. Thereafter inbox receipt, same-database projection, checkpoint and queued follow-up work are transactionally coordinated. Typed results survive restart. Unsafe operations cannot be auto-retried. Account switches and competing foreground/background workers are fenced. Foreign projections require their own receipts, not imaginary cross-database atomicity.

### US5 - One server routine contract (P1)

Handlers, generated clients and HTTP exports use the same identity, defaults, logical types, permissions and result shape. OpenAPI documents actual operations. Clients cannot supply execution principals, transactions or host capabilities. A lost mutation response is uncertain unless an authoritative idempotency receipt resolves it.

### US6 - Fresh project and coordinated release (P1)

Create untouched TS/Flutter projects, generate the current API, execute CRUD/routines/live flows, and adopt the new API in Masky without altering its domain rules. Remove obsolete aliases, old decoders and package defaults. New format restart/crash tests pass. Old clients/data are rejected, not migrated or erased. Previous Slice 1 completion records remain historical evidence only.

### US7 - Native typed RPC and in-process composition (P1)

Generated calls execute descriptor-bound operations directly through `FunctionService::invoke`, without `query("SELECT ...")` or generated `CALL`. Current-epoch JSON and binary have equivalent checked results/errors. Nested server calls share transaction, cancellation and deadlines, apply each routine's EXECUTE and INVOKER/DEFINER rules, and pass native typed values without HTTP, SQL generation or avoidable JSON envelopes. Single-data-Raft-group limits remain explicit.

### US8 - One secure, efficient codec for client/server data (P1)

Harden first; extract a lightweight core; select by measured evidence; implement RPC; converge WebSocket; consolidate fresh persistence/Raft. The same Rust codec passes golden and adversarial vectors on server, native Link, browser WASM and Dart/FRB. MessagePack is removed directly, without a transition release. Direct Arrow construction for bulk values, encode-once authorized fanout, bounded batching, connection reuse and bounded pools precede optional views. No library, language or binary format is presumed secure or fast.

### US9/US10 - Complete developer experience and verified database guarantees

Defined in [app-readiness.md](app-readiness.md). Execute the secure school/helpdesk applications (DX-01..DX-04), then validate advertised engine/SDK promises against DB-01..DB-20. Auth, tenant membership, request-scoped SSR, optimistic state, bounded pagination, files/jobs, testing/deployment, diagnostics and independent current-format restore are explicit. Optional features are documented, not silently assumed complete. A real database failure test is not replaced by an SDK mock or a shorter code sample.

## Functional requirements

Requirement IDs remain stable for task traceability; wording below is the current scope. FR-063..FR-080 are defined once in the normative [application extension](app-readiness.md).

- **FR-001:** TS/Dart MUST expose corresponding generated contracts and documented language/platform differences.
- **FR-002:** One authoritative catalog/type/naming pass MUST generate deterministic artifacts and stable entity identities.
- **FR-003:** Models/codecs/operations/facades MUST avoid accidental ORM, UI, persistence or server dependency chains.
- **FR-004:** Keys, writability, defaults, parameter modes, overloads and result cardinality MUST derive from semantic metadata.
- **FR-005:** Supported types MUST round-trip losslessly; unsupported mappings fail explicitly.
- **FR-006:** Runtime boundary checks MUST validate actual values; generic parameters and casts are not validation.
- **FR-007:** Names MUST retain exact SQL identity with deterministic collision handling and explicit schema components.
- **FR-008:** Both targets MUST provide typed CRUD/query/routine concepts.
- **FR-009:** TS table access MUST stay Drizzle-based; Dart remote access MUST remain separate from local Drift.
- **FR-010:** Named queries MUST expose analyzed parameter/result/key/cardinality and live eligibility.
- **FR-011:** Values and identifiers MUST be bound correctly without repeated ad hoc SQL rewrites.
- **FR-012:** Live descriptors MUST expose common lifecycle, freshness, errors and completion semantics.
- **FR-013:** Live changes MUST preserve full/partial/delete/filter membership and reject malformed incomplete rows.
- **FR-014:** Subscription sharing MUST respect account, query, projection, authorization and codec identity.
- **FR-015:** Snapshot chunking, history scope, presentation limit and memory limit MUST be independent.
- **FR-016:** Replay MUST use authoritative scoped delivery/cursor identities, not maximum row versions.
- **FR-017:** New-epoch Dart mirrors MUST persist row membership and progress atomically and recover after crashes.
- **FR-018:** Consumers MUST retain unapplied work and report unrepairable event-history gaps.
- **FR-019:** Foreign projections MUST supply their own idempotency receipts; no cross-database atomicity claim.
- **FR-020:** Durable actions MUST reuse current operation codecs and only retry under a proven safe policy.
- **FR-021:** Server receipts MUST bind principal/tenant, operation revision, key, canonical input digest, result and expiry.
- **FR-022:** Ordering, optimistic overlays, dependencies and worker fencing MUST survive same-epoch restarts.
- **FR-023:** Account/session ownership MUST prevent stale callbacks and cross-account cache or work leakage.
- **FR-024:** Background catch-up MUST obey global/per-consumer budgets and safely drain or cancel in-flight work.
- **FR-025:** Recovery, reset, diagnostics and retention MUST have public account-scoped APIs.
- **FR-026:** Handler/HTTP/SDK contracts MUST share validation, identity, cardinality and permissions.
- **FR-027:** OpenAPI MUST document real current-epoch paths, schemas, security and structured errors.
- **FR-028:** Generation MUST be reproducible, checked for stale artifacts and exercised in untouched projects.
- **FR-029:** This release MUST make a clean break: no old API aliases, fallback decoders, dual-stack adapters, transition window or preserved historical data requirement.
- **FR-030:** Queues, caches, pools and scheduler scans MUST be bounded; observe actual retained capacity.
- **FR-031:** Authentication, EXECUTE, RLS and role checks MUST remain server-enforced independently of descriptors.
- **FR-032:** Shared semantics MUST have compile, golden, negative, property, real-server and cross-language tests.
- **FR-033:** Specs, current architecture docs, SDK examples and generated outputs MUST match implementation status.
- **FR-034:** TS topic workers MUST not be misrepresented as Dart local synchronization.
- **FR-035:** Managed/app-constructed/attached Dart modes MUST use one session Drift connection.
- **FR-036:** Dart table generation MUST preserve qualified names and typed changes without a new ORM package.
- **FR-037:** Masky adoption MUST remove generic glue without rewriting application-specific REST/domain/UI rules.
- **FR-038:** Generated routine calls MUST bypass SQL parsing and invoke the existing function service.
- **FR-039:** Binary RPC MUST use the selected common codec; explicit typed JSON is only an external adapter over the same contract.
- **FR-040:** All boundaries MUST preserve exact numeric/temporal/array/domain semantics and distinguish omission, SQL NULL, JSON null and void.
- **FR-041:** Nested routine calls MUST be in-process and typed, with no avoidable serialization; safe owned values are sufficient.
- **FR-042:** Descriptors MUST be revision-pinned, dependency-aware and boundedly cached; mismatches fail before effects.
- **FR-043:** SQL/native calls MUST share authorization, deadlines, transaction ownership, error and staged-publication semantics; cross-group ACID and external/STREAM rollback are not invented.
- **FR-044:** Unknown outcomes MUST NOT trigger automatic mutation replay or format fallback without authoritative idempotency protection.
- **FR-045:** Performance comparisons MUST include full validation, bridges, materialization, authentication and real execution, not unchecked access-only timings.
- **FR-046:** Zero-copy is optional P2. Any implemented view MUST prove immutable ownership, bounds, projection, lifetime, cancellation and memory accounting; a copy is preferred when safer or smaller.
- **FR-047:** Use Spec 037 SQL/type/security semantics, but 038's fresh-start and P0/P1/P2 priority rules govern this delivery. Do not claim unfinished upstream ABI work complete.
- **FR-048:** Exactly one Rust-owned binary domain-value codec MUST serve backend and Link/native/WASM/FRB. One type authority; no parallel SDK serializers.
- **FR-049:** Untrusted counts/offsets/depth/work and total decoded allocations MUST be bounded before allocation/recursion/Arrow construction, including skipped and null-heavy collections.
- **FR-050:** The lean codec MUST compile without Arrow, DataFusion, RocksDB, HTTP servers or V8; backend adapters are optional and dependency direction is acyclic.
- **FR-051:** RPC, WS and owned durable/raft messages MUST share value encoding, with distinct endpoint-authorized message kinds and versioned framing.
- **FR-052:** Remove MessagePack WS/server/native/WASM/FRB paths, enum variants, defaults, negotiation and unused dependencies directly; no temporary adapter.
- **FR-053:** New owned persistence/Raft encodings MUST start fresh and remove historical readers. Test new-format recovery, corruption and crash durability with unchanged acknowledgment/quorum/fsync semantics.
- **FR-054:** Optimize total CPU/allocated/retained memory, not copy count; bounded pooling and compact copying may beat borrowed slices.
- **FR-055:** Promotion MUST meet the pre-registered memory/CPU/throughput and latency gates in the performance contract. External results are hypotheses, not KalamDB measurements.
- **FR-056:** CI MUST enforce codec ownership across backend/API/commons/Link/WASM/FRB and detect obsolete serializer dependencies. External protocols have narrow named exemptions, not a blanket exemption for old readers.
- **FR-057:** Compression MUST be off for ordinary small calls initially; any enabled compressed path has input/output/work/window limits and no permissive sniff-and-fallback decoder.
- **FR-058:** Release MUST pass hostile-frame fuzzing, property tests, arithmetic/lifetime/permission checks and lockfile advisory review; passing these is not a claim of invulnerability.
- **FR-059:** Gate D MUST compare a hardened schema-directed candidate, Postcard and Prost with identical supported semantics, exact types, limits and conversion costs; ship one winner only.
- **FR-060:** Implement or measure bounded byte/time microbatching, authorization-safe encode-once fanout, pooled connections, bulk Arrow builders and bounded buffer reuse before optional zero-copy.
- **FR-061:** New format/API/cluster epochs MUST reject old artifacts without automatic deletion or reinterpretation. Same-epoch schema evolution and durable restart remain correct.
- **FR-062:** Report CPU time/op, allocations/op, peak/retained memory, p50/p95/p99, throughput at a fixed latency SLO, wire/disk bytes and WASM footprint with pinned reproducible evidence; no cherry-picked aggregate hides a failed critical workload.

## Success criteria

SC-023..SC-032 in [app-readiness.md](app-readiness.md) add the application/stability outcomes; they are equally binding.

- **SC-001:** Paired examples compile and agree across the supported platform matrix.
- **SC-002:** Generated routine calls require no handwritten SQL, string key, serializer or registry.
- **SC-003:** A basic live query needs at most six application statements; a mirror adds one construction call.
- **SC-004:** Untouched project generation, current-schema evolution, CRUD, live, routine and handler tests pass.
- **SC-005:** Fault tests recover required changes or report an explicit gap/unknown outcome, never silent loss.
- **SC-006:** Supported types round-trip exactly; malformed/unsupported values fail explicitly.
- **SC-007:** Unchanged inputs yield byte-identical generated artifacts; failed publication leaves working files intact.
- **SC-008:** 1k/10k/100k-row and fanout workloads have reproducible timing, resource and retained-memory evidence.
- **SC-009:** Masky targets at least 50% reduction in measured generic sync glue, excluding domain/UI code.
- **SC-010:** The new cohort passes its regression suite; old clients/data are rejected without adapters or implicit reset.
- **SC-011:** Traces prove generated native routine invocation never enters SQL parsing.
- **SC-012:** SQL, explicit current JSON, binary and nested calls agree on supported values, ACL/RLS and errors.
- **SC-013:** Stale descriptors, forbidden context, cross-group writes and cancellation fail without privilege widening or unsafe retries.
- **SC-014:** Validated RPC performance meets the declared promotion gate; no assumed binary advantage.
- **SC-015:** One lean codec passes native/WASM/FRB golden vectors without server dependency inflation.
- **SC-016:** Adversarial full-path tests remain within allocation/work/depth budgets without crash, leak or bypass.
- **SC-017:** New binary WS delivers correct snapshot/change/patch/delete/ack/replay behavior in all supported SDKs.
- **SC-018:** No production MessagePack WS or obsolete first-party reader/alias remains; old protocol rejection is tested.
- **SC-019:** Memory, CPU and throughput improve on the predeclared representative workloads; critical tail-latency guardrails pass.
- **SC-020:** New persistence/Raft data survives equivalent-durability restart/crash tests; old data does not need migration.
- **SC-021:** Gate D records the selected implementation, rejected alternatives, security posture, dynamic-schema limitations and reproducible comparisons.
- **SC-022:** Every performance/security claim is marked measured, source-observed, proposed or unverified; P2 zero-copy can be deferred without blocking release.

## Delivery order and scope

Harden current code (S) -> lean shared core plus measured codec decision (C0/D) -> native RPC (B2) -> binary WS without MessagePack (W) -> fresh owned persistence/Raft (P). In parallel, start guarantee/SQL fault oracles early and complete application Gate APP plus database Gate STABLE. **Gate R and final T050 require APP/STABLE evidence (T121), not only codec benchmarks.** Core metadata/live/sync work follows [tasks](tasks.md) and its normative [T094-T121 extension](tasks-app-stability.md). Existing unsafe lifetime defects are P0 regardless of P2 deferral.

No arbitrary client workflow execution, new distributed transaction coordinator, TypeScript persistent sync package, general local relational engine or storage-engine replacement is added. Research from Kafka, TigerBeetle, Aeron/SBE, RocksDB, DuckDB and gRPC supplies scoped techniques, not a mandate to import their entire stacks. App/platform research adds ergonomic and testing patterns from Convex, Supabase, Firebase, Neon, SQLite, FoundationDB and SQLancer; no feature or superiority claim is inferred without corresponding evidence.

Related contracts: [API](contracts/public-api.md), [protocol](contracts/protocol.md), [performance and security](contracts/serialization-performance.md), [data model](data-model.md), [plan](plan.md), [tasks](tasks.md), [quickstart](quickstart.md), [research](research.md), [app readiness](app-readiness.md), [app development](contracts/application-development.md), [stability tests](contracts/database-stability.md), [app/platform research](research-app-platforms.md), [application tasks](tasks-app-stability.md).
