# Feature Specification: Unified Table Processing and Atomic Transaction Optimization

**Feature Branch**: `feat/pg-dialect`

**Created**: 2026-10-27

**Status**: Draft — requirements reviewed; implementation unverified

**Input**: Combine the uploaded storage/transaction optimization notes with the architecture review. Give storage correctness and atomic transaction optimization highest priority. Unify user, shared, and stream processing so improvements reach user and shared tables together, preserving stream behavior. Keep the current branch and PR.

**Feature directory**: `specs/039-unified-table-transactions`

**Existing PR**: [#402](https://github.com/kalamdb/KalamDB/pull/402), identified by the supplied notes. Remote status was unavailable during specification; no new branch or PR is required.

**Primary architectural acceptance gate**: User, shared, stream, and system catalog tables MUST be unified starting at the query-engine table provider boundary. System catalog tables include persisted `system.*` stores, virtual `system.*` views, `pg_catalog` relations, and Kalam-owned `information_schema` relations. User, shared, and stream tables share one provider, with SQL DML enabled according to their capability profiles. Persisted system tables stay shared-scope, hot-only, SQL-read-only, and mutable through direct typed APIs. Computed catalog relations (`pg_catalog`, virtual `system.*`, Kalam `information_schema` extensions) share that catalog provider family as in-memory projections. This unified provider layer MUST be backed by separately owned indexing, storage, flush, and transaction responsibilities, with crate boundaries adjusted where needed to eliminate parallel structures and misplaced dependencies. Eliminating duplicated maintained implementation and caller downcasting across table kinds is an essential P0 deliverable alongside snapshot safety and atomic commits. Performance improvements alone do not complete the feature if parallel provider/processing implementations remain. Stream and system-catalog consolidation at the provider/scan layer is required in the foundation, not an optional follow-up. Unification MUST NOT slow system catalog runtime paths: authentication, jobs, schema registry, type lookup, and wire catalog introspection are on the server's critical path and MUST be at least as fast as the recorded baseline (SC-015).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Commit large changes safely across unified table processing (Priority: P0)

An application updates or deletes thousands of records in user or shared tables and receives one durable, all-or-nothing transaction outcome. Maintainers implement that behavior once for both table kinds.

**Why this priority**: Snapshot correctness, atomicity, and shared user/shared processing are the foundation of every optimization. A shared-table-only fix does not complete this story.

**Independent Test**: Run identical 25,000-row insert/update/delete and mixed-operation cases against user and shared tables, including several tables in one supported commit domain. Inject late validation failure and crashes around durability.

**Acceptance Scenarios**:

1. **Given** supported tables in one commit domain, **When** a transaction changes several tables, **Then** all row/index/required-history changes become durable together in one atomic storage submission.
2. **Given** 24,999 valid changes and one invalid change, **When** commit is attempted, **Then** none become visible or durable and no success notification is emitted.
3. **Given** a reader pinned to an earlier snapshot, **When** another transaction changes a hot-only row and later flushes it, **Then** the reader continues to see its original snapshot until it ends.
4. **Given** multiple commit domains, incompatible backends, or exceeded transaction limits, **When** commit is attempted, **Then** rejection precedes any partial write.
5. **Given** an interrupted commit or replay, **When** recovery finishes, **Then** the transaction is wholly committed or wholly absent and promised effects reconcile without duplicate logical effects.

### User Story 2 - Change common behavior once for all table kinds starting from the table provider boundary (Priority: P0)

A maintainer changes query-engine table provider behavior (scan planning, filter pushdown, PK point-lookup pushdown, secondary index selection, projection, transaction overlay wrapping, statement insert/update/delete orchestration) or common row preparation, validation, batching, result handling, or notification construction in one place and gets that behavior across user, shared, stream, and system catalog tables without downcasting to concrete provider types or maintaining a copied provider per catalog relation. Processing, indexing, storage, flush, and transactions have distinct responsibilities and clean crate boundaries. Differences in ownership, retention, storage, and supported mutations remain explicit capabilities. Persisted system tables stay SQL-read-only and mutable through direct typed APIs that bypass the query engine. Computed catalog relations (`pg_catalog`, virtual `system.*` views, Kalam `information_schema` extensions) stay in-memory projections. The table-definition macro remains the only schema authoring path for those catalog relations, and the single-pass batch builder remains the SQL scan encoder.

**Why this priority**: Reducing the code maintained across user, shared, stream, and system catalog tables is essential to this feature. Unification starting from the table provider entry point, elimination of parallel provider and flush hierarchies, and clean separation of processing/indexing/storage/transaction crate responsibilities are P0 acceptance gates. Atomic transaction work must build on the shared foundation. Streams and system catalog tables must join the common provider/scan foundation without moving system runtime lookups onto the versioned user/shared engine.

**Independent Test**: Run one common provider and scan behavior suite across user, shared, stream, and system catalog tables (persisted `system.*`, virtual `system.*`, `pg_catalog`, and Kalam `information_schema`), a shared DML suite across user/shared/stream (verifying SQL DML rejection and direct-API mutation visibility on persisted system tables), and a shared versioned-mutation, hot-scan, and flush suite across user/shared. Independently test provider entry points, row processing, index change construction, storage execution, flush deduplication, and transaction coordination; then test their composition. Compare a recorded duplication inventory and production-code baseline with the final tree, verify removal of concrete provider downcasting and per-relation catalog provider copies, and verify that schema and single-pass batch macros remain and that hand-written duplicate schemas are gone. Record baseline and candidate latency for system point lookups and a representative catalog introspection query (SC-015).

**Acceptance Scenarios**:

1. **Given** two users with the same key, **When** one changes a user table, **Then** only that user's scope changes; shared tables use shared ownership but still enforce the actor's permissions.
2. **Given** configured cold storage, **When** user or shared data moves between tiers, **Then** ownership, values, versions, and results remain correct through one shared versioned flush and read lifecycle.
3. **Given** a stream table, **When** the unified table provider handles inserts, reads, deletes, and expiry, **Then** existing stream semantics remain; unsupported updates and explicit transactions stay rejected and cold storage is not silently enabled.
4. **Given** a persisted system table, **When** it is queried through SQL or mutated through its direct Rust API, **Then** SQL `SELECT` uses the direct-keyed catalog scan (primary-key equality is a point lookup, secondary predicates use the existing index, full scans encode Arrow in one pass from the macro-generated schema) with system-admin authorization enforced in-place, while SQL `INSERT`/`UPDATE`/`DELETE` and explicit transactions are rejected. The lookup does not enter version resolution, transaction overlay, or cold storage.
5. **Given** a table-provider or common-processing change, **When** it is completed, **Then** all applicable table kinds execute through the same maintained provider and processing implementation for that capability, callers interact with the unified provider contract rather than downcasting to per-table-kind structs, and no superseded equivalent copy or boilerplate catalog provider wrapper remains.
6. **Given** a storage or crate-boundary change, **When** it is integrated, **Then** query-source primitives, system catalog stores, table providers/stores, topic persistence, archive transfer, and flush orchestration each reside in their owning crate without circular or convenience dependencies.
7. **Given** an index-policy or hot-scan optimization (such as columnar hot projection, covering index aggregates, or indexed equality updates), **When** user or shared queries/mutations run, **Then** both table kinds benefit from the same implementation over scoped keys, and transaction coordination submits their data/index changes atomically.
8. **Given** consolidation is declared complete, **When** maintainers review the old-to-new responsibility map and code-size report, **Then** replaced user/shared/stream/system provider, store, flush, and applier implementations are deleted and the targeted maintained production code has a net reduction including new shared code and adapters.
9. **Given** a runtime lookup of a persisted system row by primary key or secondary index (authentication, job status, namespace, or type), **When** the server serves it, **Then** the lookup uses the direct keyed store and any in-memory cache already in front of that store, never enters query planning, and its measured median and p99 latency are less than or equal to the recorded baseline.
10. **Given** a SQL or wire client reading `pg_catalog`, `information_schema`, or a virtual `system.*` view, **When** the query runs, **Then** it uses the shared computed catalog provider, projects from memory, and its recorded introspection latency does not regress. The relation is not persisted, versioned, or flushed.
11. **Given** a persisted system table, virtual system view, `pg_catalog` relation, or Kalam-owned `information_schema` relation, **When** its schema is built, **Then** both the table definition and the Arrow schema come from the existing table-definition macro, column order matches that macro, and SQL scans use the existing single-pass batch builder. A second hand-written field list for the same relation does not remain.

### User Story 3 - Reduce transaction work without changing SQL results (Priority: P1)

An application repeatedly changes the same records, reads pending changes, uses savepoints, and commits with identical results and less redundant work.

**Why this priority**: Folding and target reuse follow proof of uncoalesced atomic commit and snapshot safety.

**Independent Test**: Exercise every commit-fold transition below, compare with unoptimized behavior, and measure repeated-key transactions and 100/1,000 reads inside dirty transactions for user/shared tables.

**Acceptance Scenarios**:

1. **Given** insert, update, savepoint, delete, rollback-to-savepoint, commit, **When** the transaction finishes, **Then** final values, intermediate affected counts, returned values, and savepoint behavior are preserved.
2. **Given** a partial update changes a filter column, **When** the transaction queries old and new predicates, **Then** newly matching rows appear, old matches disappear, and untouched fields remain intact.
3. **Given** a scanned target changes before commit, **When** the original transaction commits, **Then** stale target data cannot silently overwrite the newer state.
4. **Given** repeated changes to one key, **When** physical writes are combined, **Then** duplicate-key violations, intermediate validation, supported triggers, and promised events are preserved.

### User Story 4 - Preserve deletion, history, and live synchronization (Priority: P1)

A client reads current state, inspects deleted rows, or resumes a subscription and receives the correct view after repeated changes, storage movement, and index maintenance.

**Why this priority**: Index/history savings must not lose deletes, payloads, or promised events.

**Independent Test**: Compare primary-key, indexed, deleted-row, and history queries with authoritative results, including cold rows, offline repeated updates, reconnects, expiry, and old index contents.

**Acceptance Scenarios**:

1. **Given** a row in cold storage, **When** it is deleted, **Then** ordinary reads never resurrect it and deleted-row queries retain required prior values.
2. **Given** a deleted-row query with an indexed-column predicate, **When** ordinary secondary indexes omit tombstones, **Then** a complete authoritative path still returns correct results.
3. **Given** a retained complete-event cursor, **When** updates and a delete happened offline, **Then** all promised events replay in domain order; expired/incompatible cursors require explicit resynchronization.
4. **Given** old/new index contents coexist, **When** rebuilding, concurrent writes, or failover occur, **Then** results remain correct and rebuilt generation publication is atomic.

### User Story 5 - Stay responsive during background work (Priority: P1)

An operator flushes, compacts, exports, or imports large data while applications continue reading/writing with bounded resource use and safe cache reuse.

**Why this priority**: Contention and whole-scope buffering can amplify transaction costs.

**Independent Test**: Delay cold storage during concurrent writes; test a large shared table, one large user scope, concurrent transfers, and repeated scope churn. Record complete sustained latency/throughput/memory series.

**Acceptance Scenarios**:

1. **Given** slow segment creation/upload, **When** a same-scope write records pending work, **Then** it does not wait for bulk storage I/O; final publication remains synchronized.
2. **Given** data exceeds the work budget, **When** flushing/transferring it, **Then** processing stays within declared memory/concurrency budgets or rejects safely before exhausting resources.
3. **Given** active holders/waiters, **When** inactive coordination state is reclaimed, **Then** each active scope retains one synchronization authority and historical inactive scopes do not accumulate indefinitely.
4. **Given** warm queries on table B, **When** independent table A flushes, **Then** safe reusable state for B remains available.

### User Story 6 - Get consistent results through every SQL transport (Priority: P1)

A developer executes supported SQL through different clients and receives results appropriate to each session, including preparation, returned columns, and procedure stream operations.

**Why this priority**: Shared preparation and correct cache context prevent duplicated parsing and projection mistakes.

**Independent Test**: Alternate cold/warm requests across transports and settings; exercise quoted identifiers, comments, multiline SQL, prepared parameters, and nontrivial returned expressions.

**Acceptance Scenarios**:

1. **Given** sessions with different system-column visibility, **When** either warms the cache first, **Then** each wildcard query returns its own expected projection.
2. **Given** schema, permission, search-path, or relevant setting changes during planning, **When** old planning finishes, **Then** its result cannot be admitted or reused under the new context.
3. **Given** parsed SQL, **When** description, execution, or stream routing needs its meaning, **Then** they reuse structured semantics and agree on identifiers.
4. **Given** equivalent concurrent cold queries, **When** planning runs, **Then** unnecessary duplicate planning is suppressed and cancellation/failure does not strand waiters.

### User Story 7 - Upgrade safely and maintain clear ownership (Priority: P2)

Operators enable proven format improvements without unreadable replicas. Maintainers validate storage utilities without unrelated execution dependencies.

**Why this priority**: Format rollout and dependency cleanup follow the shared atomicity foundation; secondary-key redesign is a separately gated conditional stage.

**Independent Test**: Read mixed formats after restart, reject activation with incompatible eligible replicas, recover index rebuilding, and inspect dependency ownership and typed startup outcomes.

**Acceptance Scenarios**:

1. **Given** an eligible replica cannot read the newer format, **When** writer activation is requested, **Then** it is refused until durable cluster-wide compatibility is established.
2. **Given** mixed rows and historical schemas, **When** nodes restart or writer activation is safely disabled, **Then** stored data stays readable and the persisted format floor is honored.
3. **Given** storage/transfer code needs only its own behavior, **When** independently validated, **Then** error/configuration representation does not require the application execution layer.

### Edge Cases

- Empty transactions, insert fast paths, no-op updates, oversized commits, cancellation before/after durability.
- Same typed/composite key across tables/users; collation; explicit null versus absent field; primary-key rename collisions.
- Savepoint rewind, defaults/generated values, supported triggers/constraints, uniqueness, file replacement and vector failure.
- Long-lived snapshots and unset snapshot frontiers during update/delete, flush, compaction, reclamation, and restart.
- Cold-only targets; hot tombstones; legacy secondary tombstone entries; incomplete indexes for deleted/history views.
- Pending patches affecting predicates, joins, aggregation, authorization, ordering, projection, or limits; empty batches.
- Many small users versus one large user; failed publication/orphans; transfer cancellation; lock reclamation racing holders/waiters.
- Identical SQL under different principals, parameter types, paths, visibility, schemas; failed/cancelled cache leaders.
- Stream expiry, hard deletes, overload, unavailable history, changed incarnations, cross-domain cursors, and replay/failover.
- Incompatible replicas joining during activation, interrupted index rebuild, mixed generations, unsafe reader downgrade.

## Requirements *(mandatory)*

### Functional Requirements

#### P0 — shared processing and correctness

- **FR-001**: Query-engine table provider entry points (scan, insert, update, delete, filter pushdown, statistics, and transaction overlay wrapping) as well as common schema handling, coercion, applicable validation, batching, result handling, and notification construction MUST have one maintained implementation across user/shared/stream tables. Superseded provider structs, redundant generic provider traits, and duplicate helpers MUST be removed, not hidden behind a new facade.
- **FR-002**: User/shared tables MUST share one versioned mutation, read, hot-scan, and flush lifecycle over a unified scoped key/row representation. Applicable transaction optimizations and hot-storage read/write fast paths (including columnar hot projection, covering index aggregates, and indexed equality updates) MUST ship and be tested for both together; direct multi-row and insert fast paths MUST remain equivalent.
- **FR-003**: Ownership MUST distinguish required user scope from explicit shared scope independently of authenticated actor. Missing user scope fails closed; shared ownership never bypasses authorization/impersonation controls.
- **FR-004**: Cold-storage capability MUST be independent of ownership. User/shared retain configured hot/cold behavior; streams retain storage choices, expiry, hard deletion, update rejection, and transaction restrictions.
- **FR-005**: Uncommitted data MUST remain transaction-local and invisible to other sessions. Rollback/savepoint rollback MUST leave no uncommitted durable row/index/history changes.
- **FR-006**: One accepted nonempty transaction MUST use one atomic storage submission covering all touched tables, main rows, indexes, required history, and durable effect intents in its supported domain. Per-row/per-table commits MUST NOT substitute for group atomicity.
- **FR-007**: Preparation MUST validate all mutations before physical application without deleting files, publishing success, or other visible effects. The initial atomic implementation MUST preserve uncoalesced semantics for comparison.
- **FR-008**: Cross-domain/incompatible-backend transactions and exceeded byte/row/version limits MUST fail before partial writes and MUST NOT be silently split. Mixed user/shared transactions qualify only where existing routing permits one compatible domain.
- **FR-009**: Snapshot reads MUST preserve their committed frontier across later writes and storage movement. Needed versions MUST survive reader/history retention. Failing repeatable-read regression blocks release.
- **FR-010**: Replay MUST deterministically produce the same versions/data from identical committed input and prior state. Ordering MUST remain domain-scoped and independent of unordered iteration or divergent local settings.
- **FR-011**: Constraints, authorization, uniqueness, and write conflicts MUST remain enforced. Reused scanned targets MUST be checked against authoritative committed versions; target handles are not authorization tokens.
- **FR-012**: Durable success MUST precede success notifications and destructive cleanup. File/vector/manifest/topic/live effects MUST recover idempotently under existing delivery contracts; acknowledgement MUST meet promised durability/publication guarantees.

#### P1 — transaction efficiency and equivalent results

- **FR-013**: Commit folding MUST identify targets by table, explicit scope, and canonical typed primary key. Original statement validation/results/counts, savepoint history, and required events MUST remain intact until their semantics are resolved.
- **FR-014**: Folding MUST implement the transition table below and preserve untouched fields, explicit null, supported conflict handling, defaults, generated values, triggers, unique-key changes, key renames, and file replacement.
- **FR-015**: Authoritative target loading and bulk primary-key validation MUST reuse valid work per distinct scoped target with bounded lookup. Committed/staged uniqueness, tombstones, and cold-only rows MUST be considered; repeated loads and transmitted preimages MUST be measured and justified.
- **FR-016**: Dirty-transaction reads MUST merge inserts/updates/deletes while preserving full partial-patch semantics, authorization, joins, aggregates, order, and limits. Both directions of predicate movement MUST work.
- **FR-017**: Overlay reads MUST avoid whole-table row materialization and unnecessary columns when bounded columnar processing preserves semantics. Shared payloads MUST preserve savepoint isolation and accurate retained-memory accounting.
- **FR-018**: Complete-event replay and latest-state-since-cursor reads MUST be distinct contracts. Folding/reclamation MUST preserve existing complete-event guarantees and retention. Expired, overloaded, or incompatible history MUST explicitly signal gap/resynchronization.
- **FR-019**: Deleted rows MUST retain full values required by queries/change delivery. Primary-key lookup MUST recognize the latest tombstone without resurrecting cold data.
- **FR-020**: New tombstones MUST be omitted from ordinary secondary indexes only once compatible scanning is established. Index roles MUST be explicit; deleted/history queries MUST use complete authoritative paths and residual predicates.
- **FR-021**: Legacy/new index contents MUST coexist safely. Rebuild/cleanup MUST capture concurrent changes, validate consistency, atomically publish generations, and recover after failure. Snapshot/history retention remains independent of index policy.

#### P1 — bounded background work and shared query semantics

- **FR-022**: Bulk flush preparation, encoding, transfer, and cold I/O MUST not hold the scope publication lock needed by ordinary writes. Final publication MUST preserve unique segment identity, pending writes, durable-before-cleanup order, and orphan/retry recovery.
- **FR-023**: Flush, compaction, and transfers MUST have end-to-end memory/concurrency budgets covering retained rows, version keys, output buffers, and concurrent jobs. Batched scans alone do not establish bounded processing.
- **FR-024**: Archive compression/blocking storage work MUST not monopolize request-serving execution capacity. Transfers MUST preserve scope/path validation and failure/cancellation cleanup.
- **FR-025**: Coordination entries MUST have safe reclamation so inactive historical scopes do not grow indefinitely. All holders/waiters for an active scope MUST share one synchronization authority.
- **FR-026**: Reuse MUST distinguish syntax, schema/authorization/session-sensitive metadata, and storage-sensitive state. Flush/compaction MUST invalidate affected dependencies while retaining safe unrelated state; stale in-flight work cannot re-enter current caches.
- **FR-027**: Cache identity MUST cover every semantic context dimension affecting results: visibility, relevant settings/search path, parameter types, authorization, and schema dependencies. Principal-bound state MUST be isolated.
- **FR-028**: Equivalent concurrent cache misses MUST coordinate planning with bounded waiters and safe failure/cancellation. Budgets MUST account for retained payloads as well as entries.
- **FR-029**: Parsing, preparation, binding, result description, returned-expression planning, and routing MUST reuse structured SQL semantics. Stringification/reparsing and manual identifier scanning MUST not create another interpretation path.

#### P2/P3 — compatibility, ownership, and evidence

- **FR-030**: New row-format writing MUST require a durable compatibility barrier for all replicas eligible to apply it. Mixed formats, historical schemas, projected reads, full tombstones, restart, and safe writer disable MUST be verified first.
- **FR-031**: Row-format rollout and stable-primary-key secondary-index redesign MUST remain independently gated migrations. The latter is conditional on measured benefit and separate catch-up/publication validation; unchanged indexed values do not authorize skipping version-keyed maintenance.
- **FR-032**: Existing data, table identity, ownership placement, version domains, and public contracts MUST remain compatible. No automatic destructive migration, silent repartitioning, or unsupported downgrade is permitted.
- **FR-033**: Storage I/O, key-value operations, table processing, transaction authority, query semantics, and scheduling MUST have clear owners. Transfer errors/storage descriptions MUST not force unrelated application-execution/catalog-provider dependencies.
- **FR-034**: Composition MUST provide typed, validated job capabilities and initialization outcomes. Orchestration MUST preserve shared context ownership without runtime type guessing for ordinary access.
- **FR-035**: Tests MUST cover user/shared together and streams for applicable common behavior. Constraints, auth, SQL, snapshots, live delivery, file/vector effects, and direct batch paths MUST not regress.
- **FR-036**: Evidence MUST distinguish baseline observations, proposals, implementation, and measurement. Report client/server stages, submissions, target loads, operations/bytes, retained memory, scheduling/encoding work, and each relevant runtime in seconds.
- **FR-037**: Delivery MUST retain the current branch/PR, preserve unrelated work, and activate in dependency order. Update relevant architecture, public behavior, and operational documentation before shipping.

#### P0 — required composable table codebase

- **FR-038**: The table codebase MUST unify user, shared, stream, and system catalog tables starting at the query-engine table provider boundary, separating provider entry points, common row processing, index semantics, physical storage, flush deduplication, and transaction coordination into independently testable responsibilities. System catalog tables include persisted `system.*` stores, virtual `system.*` views, `pg_catalog` relations, and Kalam-owned `information_schema` relations. Table kind MUST select explicit capabilities, scope mode, and storage engine (versioned MVCC vs append stream log vs direct-keyed system store vs computed catalog projection); it MUST NOT select a copied end-to-end provider or applier implementation.
- **FR-039**: Common provider and row processing MUST own query-engine scan/insert/update/delete entry points, filter pushdown, projection, row preparation, coercion, applicable constraint evaluation, batching, result shaping, and change-event construction once. Callers across execution, applier, jobs, and bridges MUST interact with the unified provider surface rather than downcasting `Any` to separate user/shared/stream provider structs. Authorization decisions (including built-in system-admin scan guards on system tables) remain enforced for every applicable operation without extra wrapper providers.
- **FR-040**: Indexing MUST own typed key extraction, uniqueness/lookup semantics, index roles, and index-change construction once for equivalent capabilities over scoped and direct keys. Separate user vs shared primary-key index wrapper types and codecs MUST be consolidated where underlying prefix and row encodings are shared. Indexing MUST preserve scope and tombstone/history rules and contribute changes to the encompassing atomic commit rather than committing independently.
- **FR-041**: Storage and flush MUST own physical read/write, encoding, key/file layout, tier access, and hot-to-cold deduplication within existing storage ownership rules. User and shared tables MUST share one versioned store, codec, vector-staging, and flush implementation parameterized by scope; stream tables retain their localized append-log store; and persisted system tables use one shared direct-keyed store that supports SQL `SELECT` scans (primary-key equality as a point lookup, secondary index selection, limit, and projection) and direct typed Rust CRUD APIs that bypass the query engine, without cold Parquet flush or MVCC system columns. Runtime callers (authentication, jobs, schema registry, catalog services) MUST keep calling that direct store API. SQL scans of persisted system tables MUST encode Arrow in one pass from the typed value and the macro-generated schema. They MUST NOT convert through a generic scalar row, version-resolution pass, transaction overlay, or cold segment. Computed catalog relations MUST NOT be written to that store.
- **FR-042**: Transaction coordination and DML application MUST own staging, snapshots, conflict validation, savepoints, atomic application, and post-commit effect ordering under the existing single durable authority. User and shared transaction batch preparation and application MUST use one unified codepath rather than parallel user and shared applier loops. Table adapters MUST NOT introduce independent commit paths.
- **FR-043**: Scope, index availability, mutation support, storage tiers, retention, and transaction participation MUST remain distinct capabilities with validated combinations. Stream and system catalog reuse MUST include the unified provider/scan substrate and all applicable shared processing. Stream differences (expiry, hard deletion, local version generation, unsupported updates), persisted system differences (SQL DML rejection, direct API mutation, non-MVCC point keys, no cold flush), and computed catalog differences (in-memory projection, no store, no flush) MUST stay localized in their engines rather than spreading table-kind branches or per-relation provider copies through the codebase.
- **FR-044**: Planning MUST inventory duplicated provider, store, flush, system-catalog provider, `pg_catalog` view, virtual system view, and applier code, map each duplicated responsibility to its final owner (including crate-boundary adjustments), and define deletion targets before implementation. Completion MUST remove superseded production paths, redundant provider traits, per-relation catalog provider wrappers, and hand-written schemas that duplicate the table-definition macro. The table-definition macro and the single-pass batch-construction macros MUST be kept; they are the schema source and the hot scan encoder, not duplication to delete. Retained specialized behavior MUST have a documented semantic or measured performance reason and dedicated coverage.
- **FR-045**: Consolidation MUST demonstrate net reduction in targeted maintained production code after counting new common implementation and adapters. File moves, renames, generated-code changes, test removal, and documentation deletion MUST NOT count as reduction. A facade over the old providers does not satisfy this requirement; common behavior changes must be implemented once.
- **FR-046**: Crate boundaries MUST be realigned where misplaced responsibilities cause coupling or duplication without introducing circular dependencies: shared query-source, deferred-scan, and DML plan primitives belong in `kalamdb-datafusion-sources` (shared by both `kalamdb-system` and `kalamdb-tables` so `kalamdb-system` never depends on `kalamdb-tables`), durable topic message storage belongs in `kalamdb-publisher` rather than `kalamdb-tables`, archive transfer utilities in `kalamdb-transfer` MUST NOT depend on `kalamdb-core`, and versioned flush jobs in `kalamdb-flush` MUST NOT duplicate user versus shared table flush pipelines. The system-table schema macro stays in `kalamdb-macros`. Persisted system stores stay in `kalamdb-system`. Computed `pg_catalog`, virtual `system.*`, and Kalam `information_schema` projections stay in `kalamdb-views`. Neither catalog crate depends on `kalamdb-tables`.
- **FR-047**: System catalog performance is a hard gate, separate from the user/shared 5% allowance in SC-009. Direct keyed reads and writes used by authentication, jobs, schema registry, and catalog services MUST NOT enter query planning, version resolution, overlay merge, or cold storage, and MUST keep the in-memory caches already in front of those stores. SQL primary-key equality on a persisted system table MUST be a point lookup. `pg_catalog`, virtual `system.*` views, and Kalam-owned `information_schema` relations MUST share one computed catalog provider for access checks, projection, limit, and schema, while each relation keeps its specialized in-memory projection. Stock DataFusion `information_schema` behavior that Kalam does not replace stays on that provider. Measured median and p99 latency for the recorded point-lookup suite and the recorded catalog-introspection query MUST be less than or equal to baseline. Any regression blocks activation. The table-definition macro is the only authoring path for these schemas: it emits the table definition, the memoized Arrow schema, the primary-key column, and the column order consumed by the single-pass batch builder. Hand-written Arrow field lists and hand-built column-definition lists for the same relations are removed.

### Required Responsibility Separation

These responsibilities describe the required ownership outcome across the provider, processing, indexing, storage, flush, and transaction layers.

| Responsibility | Owns | Collaboration requirement |
| --- | --- | --- |
| Unified table provider & common processing | Query-engine scan/insert/update/delete entry points, filter pushdown, overlay wrapping, row preparation, common validation, batching, result and change-event construction | One maintained provider and processing path for user/shared/stream; callers use unified provider API without per-kind downcasting |
| System catalog | Persisted direct-keyed stores, computed `pg_catalog` / virtual `system.*` / Kalam `information_schema` projections, macro-generated schema, single-pass scan encoding | Runtime point lookups stay on the direct store; computed relations stay in memory; schemas come from the table-definition macro; latency meets SC-015 |
| Indexing | Scoped typed keys, index roles, lookup and uniqueness semantics, index-change construction | One prefix/PK index and codec implementation across user/shared scopes; changes join the same atomic commit as rows |
| Storage & flush | Physical data access, encoding, key/file layout, tiers, versioned hot-to-cold flush, and stream append-log lifecycle | One versioned store/flush engine for user/shared (grouped by scope) and one stream log engine; no duplicated SQL or transaction policy |
| Transaction coordination & applier | Staging, snapshots, conflicts, savepoints, unified versioned commit preparation, commit atomicity, effect ordering | One existing durable authority and one versioned apply pipeline; supported operations consume shared preparation |
| Table capabilities and scope | User/shared ownership, supported mutations/indexes/tiers, RLS mode, retention and transaction participation | Declarative capability profile per table kind; actor identity remains separate from data ownership |
| Crate boundary hygiene | Clean separation between query-source primitives, table providers/stores, flush, topic publishing, and archive transfer | No reverse dependency from transfer to core or topic persistence inside tables |

### Required Commit-Fold Semantics

| Earlier state | Next operation | Required final-state behavior |
| --- | --- | --- |
| Missing | Insert | Insert after absence and uniqueness validation |
| Insert | Update | Insert merged values; preserve update statement result |
| Update | Update | Merge patches in order, including explicit null |
| Insert | Delete | No final live row for previously absent key; preserve promised events |
| Update | Delete | Full last-before-delete payload and required tombstone/history |
| Delete | Insert | Validated reinsertion with history and constraints preserved |
| Insert | Insert | Duplicate-key error unless explicit conflict handling resolves it |
| Delete | Update | Existing SQL error/zero-row behavior; never invent a partial row |
| Delete | Delete | One effective final deletion with original statement results |

Insert-delete-reinsert, key renames, multiple scopes, savepoint rewind, and required intermediate effects MUST compose correctly. Folding final state does not imply folding complete event history.

### Key Entities *(include if feature involves data)*

- **Table scope**: Table identity plus required user ownership or shared ownership, distinct from actor identity.
- **Processing capability**: Supported mutations, retention, tiers, and transactions for a table kind.
- **Mutation target**: Canonical typed key, scope, authoritative prior state, schema identity, and expected version.
- **Prepared transaction**: Validated changes for one commit domain, original SQL results, and recoverable effect intents.
- **Snapshot frontier / version domain**: Committed visibility boundary and exact history/ownership domain within which versions are ordered.
- **Pending overlay**: Isolated transaction-local changes with savepoint and partial-patch meaning.
- **Tombstone / retained change**: Deletion or event with payload required by its query/delivery contract.
- **Publication generation**: Committed segment/index/cache dependency identity preventing stale publication.
- **Compatibility floor**: Durable minimum reader/writer capabilities for stored and replicated data.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: User/shared each pass 25,000-row insert/update/delete and mixed-table same-domain cases with one atomic storage submission per supported nonempty commit, zero partial outcomes at every tested failure point, and correct original affected counts.
- **SC-002**: Snapshot values remain identical across concurrent updates/deletes, flush, and compaction for hot-only/cold-only/mixed tiers. Conflict, rollback, savepoint, constraint, and authorization tests have zero regressions.
- **SC-003**: Every fold transition and extended edge case passes for user/shared. For 25,000 operations on one key, final-state storage work is bounded by distinct targets and affected indexes; separately promised history is counted and preserved.
- **SC-004**: Promised deletes, predicate exits, and repeated offline changes are delivered or explicitly reported as a gap. Replay creates zero duplicate logical durable effects without inventing universal exactly-once network delivery.
- **SC-005**: Every inventoried common provider, row-processing, versioned-store, flush, and applier responsibility has one production implementation used by all applicable table kinds. Superseded user/shared/stream provider structs, redundant generic provider traits, duplicate PK index/codec wrappers, duplicate user/shared flush jobs, and parallel user/shared transaction applier loops are deleted. Callers across execution, applier, jobs, and bridges use the unified provider surface without per-kind `Any` downcasting. Report removed, added, and net maintained production lines for the fixed consolidation scope, including new shared code/adapters. The net reduction must be positive; no assumed reduction percentage is claimed.
- **SC-006**: At least three complete 10-minute mixed-workload runs after warmup remain within budgets declared before measurement. Increasing one scope or scope churn by 10x stays within the same work budget plus 10% measurement tolerance; inactive coordination entries return to their declared steady-state bound after the documented reclamation interval.
- **SC-007**: During a deliberately delayed 1-second bulk cold-storage operation, ordinary pending-write registration does not wait on its bulk-operation lock. Flushing table A causes zero invalidations of demonstrably independent warm entries for B; affected data/schema changes stay visible.
- **SC-008**: Transport/session cache cases return correct projection/values in both warmup orders with zero stale-context reuse. Equivalent concurrent misses avoid duplicate reusable planning; cancelled/failed leaders release waiters safely.
- **SC-009**: Each enabled optimization shows positive median improvement exceeding measured baseline run-to-run spread in its declared target metric across at least five matched repeats. Unaffected user/shared/stream throughput and p95/p99 latency regress by no more than 5%; unexplained regressions block activation. This 5% allowance does not apply to system catalog runtime paths, which use SC-015. Report all results with fixed data/hardware/durability/build settings and runtimes in seconds.
- **SC-010**: 100-read and 1,000-read dirty-transaction workloads preserve exact results while avoiding whole-table scalar-row conversion on the optimized path; memory/allocation savings are measured.
- **SC-011**: Mixed-format/index-generation restart, failover, rebuild, and eligible-replica tests have zero unreadable rows or lost deletes. Every incompatible-replica activation is rejected and row/index migrations are independently controlled.
- **SC-012**: All maintained affected transaction/storage/SQL, parity, stream, fault-injection, upgrade, and applicable CI checks pass before merge. Requirements-checklist completion is not implementation or performance acceptance.

- **SC-013**: The unified table provider, row processing, indexing, storage/flush, and transaction responsibilities each pass independent contract tests and composed behavior tests. One table-provider or common-processing correction is verified across all three table kinds without parallel edits to equivalent provider logic; one index-rule or hot-scan optimization (columnar hot scan, covering index aggregate, or indexed equality update) is verified across both user and shared tables from the single shared implementation.
- **SC-014**: The consolidation inventory has zero unexplained retained duplicate paths and zero misplaced crate dependencies (transfer does not depend on core; topic persistence lives in the publisher crate rather than tables; system and views crates do not depend on tables). Every supported capability combination has parity coverage, unsupported combinations fail explicitly, and measured runtime/allocation results meet SC-009 and SC-015 rather than trading code reduction for unchecked overhead.
- **SC-015**: Before changing system catalog code, record a `dev`-profile baseline for (a) direct primary-key and secondary-index lookups used by authentication, jobs, namespaces, and types, and (b) one representative catalog introspection query covering `pg_catalog` and `information_schema` (the wire client's namespace/table/column lookup). After the change, at least five matched repeats show median and p99 latency less than or equal to that baseline for both suites. Report each runtime in seconds. A SQL primary-key predicate on a persisted system table is a point lookup in the plan, not a full scan. The table-definition macro is the only remaining schema authoring path for persisted system tables, virtual system views, `pg_catalog` relations, and Kalam-owned `information_schema` relations; the single-pass batch builder remains the SQL scan encoder. Hand-written duplicate schemas for those relations are gone. Provider-impl macros that only copied `TableProvider` are gone.

## Assumptions

- This invocation creates specification artifacts, not production code, a commit, or a new PR. Existing dirty changes may address earlier findings; planning must recheck current source before assigning work.
- Highest priority combines Stories 1 and 2: common processing across user/shared/stream, separate indexing/storage/transaction responsibilities, and unified user/shared atomic application are one foundation. Stream participation in common processing is mandatory; new stream updates/cold archival/shared ownership and cross-domain transactions are outside scope. System catalog tables join the same foundation as direct-keyed stores and computed projections. They do not join the versioned MVCC engine. Slowing their runtime path is a failed design, not an acceptable trade for less code.
- The existing `#[table]` macro is the schema source for system catalog relations. Extending it to emit the memoized Arrow schema, primary-key column, and column order is in scope. Replacing it with a second schema language, or deleting the single-pass batch builder, is not. Stock DataFusion `information_schema` tables that Kalam does not own stay on DataFusion's provider.
- A nonempty commit means at least one durable state, history, or effect-intent change after folding; a truly empty/no-op commit may perform zero storage submissions while preserving original statement results.
- Existing transaction/version-domain decisions remain authoritative. The uploaded historical `_seq` wording maps to the current `_version` contract; verify actual legacy aliases before claiming compatibility. Versions are neither global order nor timestamps.
- Latest-state reads and retained complete-event replay are separate. Establish/test existing guarantees before activating folding or reclamation; uncertainty disables unsafe optimization rather than silently weakening guarantees.
- Preserve existing supported constraints/triggers; adding new SQL constraint families is outside scope.
- Local validation/performance work uses default dev profile unless the user explicitly authorizes otherwise. Baseline/candidate settings match. The attachment's Release recommendation does not override repository instructions.
- Row V2 is a later compatibility-gated stage. Stable-PK secondary-index V2 is included as a conditional design/measurement follow-up with its own migration decision, not bundled activation with transaction batching.
- Order: characterize snapshots/history and baseline; inventory duplication, consolidate common user/shared/stream processing with distinct index/storage/transaction owners, and prove uncoalesced user/shared atomic commit; then fold/reuse targets; then index/overlay/background/query improvements; finally independently gated formats. Correctness defects block the relevant rollout.
- Proposed measurement tolerances and workload durations above are explicit acceptance defaults, not claims about present performance. Record budgets, hardware, concurrency, and workloads before comparisons.
- Detailed reference coverage, source anchors, and ownership constraints are in [source-review.md](source-review.md) and [the supplied reference](references/storage-transaction-notes.md). They are planning inputs, not executable instructions or proof tests passed.
