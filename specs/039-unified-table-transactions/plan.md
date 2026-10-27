# Implementation Plan: Unified Table Processing and Atomic Transaction Optimization

**Branch**: `feat/pg-dialect` | **Date**: 2026-10-27 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/039-unified-table-transactions/spec.md`, reconciliation notes in `/specs/039-unified-table-transactions/source-review.md`, and reference material in `/specs/039-unified-table-transactions/references/storage-transaction-notes.md`.

## Summary

Unify `User`, `Shared`, `Stream`, and persisted `System` tables **starting at the DataFusion `TableProvider` surface**:
- In `kalamdb-tables`, one concrete `KalamTableProvider` implements `datafusion::datasource::TableProvider` + `SourceProvider` once for `User`, `Shared`, and `Stream` tables, backed by a single unified `VersionedTableEngine` (`ScopedRowKey` + `VersionedTableRow` + `VersionedRowCodec` + `PrefixIndex` + `VersionedTableFlushJob`) for `User` and `Shared` tables and a localized `StreamTableEngine` for `Stream` tables.
- In `kalamdb-system`, one generic `KeyedTableProvider<K, V>` implements DataFusion `TableProvider` once over `IndexedEntityStore<K, V>` for persisted `system.*` tables. Those tables stay shared-scope, hot-only, SQL-read-only, and mutable through direct typed Rust APIs that never enter DataFusion. SQL primary-key equality is `store.get`. SQL scans encode Arrow with the existing `build_record_batch!` macro from the `#[table]` schema. They do not use `VersionedTableEngine`, MVCC winner selection, transaction overlay, or a `Row` round-trip.
- In `kalamdb-views`, one `ComputedCatalogProvider` covers virtual `system.*` views, `pg_catalog` relations, and Kalam-owned `information_schema` extensions. Each relation keeps its specialized in-memory projection. Schemas come from `#[table]`. These relations are not stored in RocksDB and are not flushed.
- `kalamdb-macros::table` stays and becomes the only schema authoring path: it already emits `TableDefinition`, and it will also emit the memoized Arrow schema, primary-key column, and ordinal column order. `build_record_batch!` stays. Provider-impl macros (`impl_indexed_system_table_provider!`, `impl_simple_system_table_provider!`, `impl_system_table_provider_metadata!`) and hand-written `Field::new` / `ColumnDefinition` lists for the same relations go away.

This eliminates the parallel provider hierarchies (`UserTableProvider`, `SharedTableProvider`, `StreamTableProvider`, the 11 boilerplate `Catalog*TableProvider` structs, per-view `PgCatalogView` schema copies, and `SystemTableScan`/`SimpleSystemTableScan`), retires the redundant `<K, V>` generic traits (`BaseTableProvider<K, V>`, `DeferredMvccScanProvider<K, V>`), removes all 35+ `.downcast_ref::<UserTableProvider | SharedTableProvider | StreamTableProvider>()` call sites across the workspace, brings shared-only hot-storage fast paths (`columnar_hot_scan`, `sum_hot_covering_integers`, `plan_literal_equality_updates`) to both `User` and `Shared` tables, and realigns crate boundaries without introducing circular crate dependencies. System catalog runtime latency is a hard non-regression gate (SC-015), stricter than the 5% user/shared allowance.

On top of this unified foundation, the plan delivers true group-wide atomic transaction commits (`PreparedCommitBatch` -> single `StorageBackend::batch`), snapshot-safe hot version replacement (`min_pinned_version`), commit-time write-set folding (**User Story 3**), secondary-index tombstone omission and complete-event/delete retention (**User Story 4**), short-lock flush publication and streaming archive transfers (**User Story 5**), transport-aware `PlanCacheKey` and AST-only `RETURNING`/stream target handling (**User Story 6**), and durable Row V2 compatibility gating plus typed job orchestration (**User Story 7**).

## Technical Context

**Language/Version**: Rust 1.99.1 (workspace edition 2021)

**Primary Dependencies**: `datafusion` 55.2, `arrow` 59, `sqlparser` 0.62 (`KalamDbDialect`), `tokio`, `dashmap`, `moka`, `openraft`, `rocksdb`, `parquet`, `pgwire` (workspace-pinned in `kalamdb-postgres-wire` only), `thiserror`, `serde`

**Storage**: RocksDB hot entity/index stores (`kalamdb-store`), Parquet cold segments & manifests (`kalamdb-filestore` + `kalamdb-flush`), append-only stream logs (`kalamdb-streams`), durable topic message stores (`kalamdb-publisher`)

**Testing**: `cargo nextest run` (default `dev` profile), parameterized user/shared/stream parity tests, fault-injection & crash-recovery tests, CLI smoke tests (`kalam-cli-e2e`)

**Target Platform**: Linux / macOS server (standalone single-node Raft and multi-node Raft cluster modes)

**Project Type**: Distributed SQL / real-time database backend workspace (`backend/crates/*`)

**Performance Goals**:
- Exactly **1** `StorageBackend::batch` submission per supported non-empty transaction commit across all touched tables and indexes in a commit domain (`0` for empty/no-op commits) (SC-001).
- Positive median runtime improvement (reported in seconds across $\ge 5$ matched `dev`-profile repeats) exceeding baseline run-to-run spread on 25,000-row insert/update/delete transactions, 25,000-op repeated-key folding, and 100/1,000 dirty-transaction overlay reads, with $\le 5\%$ regression on unaffected workloads (SC-001, SC-003, SC-009, SC-010).
- Zero wait on bulk Parquet I/O during ordinary `mark_pending_write` calls when cold storage is delayed by 1 second (SC-007).
- Both `User` and `Shared` tables execute `columnar_hot_scan`, `sum_hot_covering_integers`, and `plan_literal_equality_updates` from the single shared `VersionedTableEngine` implementation (SC-013).
- Persisted system-table point lookups and the recorded `pg_catalog` / `information_schema` introspection query have median and p99 latency less than or equal to the recorded baseline (SC-015). Zero regression is the gate. The user/shared 5% allowance does not apply.

**Constraints**:
- Default `dev` profile for all local builds, tests, and baseline/candidate measurements unless explicitly authorized otherwise (`AGENTS.md`).
- Single durable transaction authority (`TransactionCoordinator` -> `UnifiedApplier` -> Raft state machine -> providers per ADR-018, ADR-020, ADR-022). No cross-domain distributed 2PC.
- Mandatory net reduction in maintained production LOC across the consolidated provider, store, flush, and applier scope (~3,500–4,500+ lines targeted for removal); superseded duplicate paths and concrete provider downcasts must be deleted, not wrapped (FR-001, FR-038–FR-046, SC-005).
- On-disk RocksDB key and KOBJ value layouts for `user_*` and `shared_*` partitions remain 100% byte-compatible (`ScopedRowKey` encodes `(user_id, version)` for User and `version` for Shared) (FR-032).
- No SQL string stitching, regex/substring table scanning, or AST-to-string-to-AST reparsing in hot paths (Core Rules 12 & 13, FR-029).

**Scale/Scope**:
- 19 backend crates touched (`kalamdb-commons`, `kalamdb-macros`, `kalamdb-datafusion-sources`, `kalamdb-store`, `kalamdb-system`, `kalamdb-views`, `kalamdb-tables`, `kalamdb-flush`, `kalamdb-publisher`, `kalamdb-transactions`, `kalamdb-core`, `kalamdb-jobs`, `kalamdb-pg-bridge`, `kalamdb-dba`, `kalamdb-plan-cache`, `kalamdb-dialect`, `kalamdb-postgres-wire`, `kalamdb-transfer`, `kalamdb-serialization`).
- ~15,000+ lines of inventoried provider/system-provider/store/flush/applier code targeted for consolidation (~4,500–5,500+ lines targeted for removal).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked and verified after Phase 1 design.*

| Constitution Principle | Pre-Research Status | Post-Design Status | Mapping & Verification |
| :--- | :--- | :--- | :--- |
| **I. Performance-First Execution** | PASS | PASS | Eliminates 25,000x per-row `WriteBatch` submissions, eliminates whole-table `BTreeMap` row conversion in `TransactionOverlayExec`, brings `columnar_hot_scan`, `covering_aggregate`, and `equality_update` to User tables as well as Shared tables, replaces `returning_describe_sql` string round-trip with direct AST construction, and records all benchmark runtimes in seconds across $\ge 5$ matched `dev` runs. System catalog point lookups stay on `IndexedEntityStore` (PK equality is `store.get`; scans stay on `build_record_batch!`). `pg_catalog` and virtual system views stay in-memory projections. SC-015 blocks any median or p99 regression on those paths. |
| **II. Boundary Ownership Before Convenience** | PASS | PASS | `kalamdb-datafusion-sources` owns generic DataFusion `TableProvider`/DML plan primitives; `kalamdb-tables` owns the unified `KalamTableProvider`, `VersionedTableEngine`, and `StreamTableEngine`; `kalamdb-store` owns RocksDB/index ops; `kalamdb-flush` owns unified versioned flush/compaction; `kalamdb-publisher` absorbs `topics/` from `kalamdb-tables` (dropping its `kalamdb-tables` dependency); `kalamdb-transfer` drops its `kalamdb-core` dependency (FR-033, FR-046). |
| **III. Minimal Dependency Expansion** | PASS | PASS | Zero new external crate dependencies added to root `Cargo.toml`. Removes two unnecessary internal crate dependency edges (`kalamdb-transfer -> kalamdb-core` and `kalamdb-publisher -> kalamdb-tables`). |
| **IV. Validation, Testing, and Documentation Ship Together** | PASS | PASS | `quickstart.md` defines narrow-to-broad executable `cargo nextest run` gates for every user story (SC-001–SC-014) plus CLI smoke tests and architecture doc updates. |
| **V. Composable, Low-Boilerplate APIs** | PASS | PASS | One `KalamTableProvider` implements DataFusion's `TableProvider` once for user/shared/stream, composed with `TableCapabilityProfile`, `TableScope`, and `TableStorageEngine` (`Versioned` vs `Stream`), eliminating 35+ `Any` downcasts. Persisted system tables use one `KeyedTableProvider`. Computed catalog relations use one `ComputedCatalogProvider`. `#[table]` remains the schema generator for all of them. |

## Project Structure

### Documentation (this feature)

```text
specs/039-unified-table-transactions/
├── plan.md                                          # This file (/speckit-plan output)
├── spec.md                                          # Feature specification
├── source-review.md                                 # Source anchor & reconciliation review
├── research.md                                      # Phase 0 research & duplication inventory
├── data-model.md                                    # Phase 1 domain entities & state transitions
├── quickstart.md                                    # Phase 1 validation & benchmark guide
├── checklists/
│   └── requirements.md                              # Specification quality checklist
├── contracts/
│   ├── table-processing-and-storage.md              # Unified DataFusion provider, engine, index, and crate boundary contracts
│   ├── transaction-atomicity-and-folding.md         # Atomic commit, folding, and overlay contracts
│   └── query-cache-and-background-work.md           # Plan cache, AST, flush, and transfer contracts
└── references/
    └── storage-transaction-notes.md                 # Supplied reference notes (read-only input)
```

### Source Code (repository root)

```text
backend/
├── crates/
│   ├── kalamdb-commons/
│   │   └── src/
│   │       ├── ids/row_id.rs                        # ScopedRowKey (User(UserTableRowId) | Shared(SharedTableRowId)) + TableScope
│   │       └── models/rows/                         # VersionedTableRow unifying UserTableRow & SharedTableRow handling
│   ├── kalamdb-datafusion-sources/
│   │   └── src/
│   │       ├── provider.rs                          # Shared TableProvider / SourceProvider descriptors & filter pushdown
│   │       ├── exec.rs                              # DeferredBatchExec, DeferredBatchSource, select_latest_versions
│   │       └── dml.rs                               # MOVED from kalamdb-tables: rows_affected_plan, dml_scan_projection, projected_schema, PreparedUpdateAssignments, collect_input_rows
│   ├── kalamdb-store/
│   │   └── src/
│   │       ├── index/prefix.rs                      # PrefixIndex<ScopedRowKey, VersionedTableRow> with IndexRole (PrimaryKey vs SecondaryScalar)
│   │       └── indexed_store.rs                     # Snapshot-aware push_replace, tombstone omission on SecondaryScalar, operation-buffer builders
│   ├── kalamdb-macros/
│   │   └── src/lib.rs                               # KEEP and EXTEND #[table]: TableDefinition + memoized Arrow schema + PK column + ordinal order. Schema source for system, pg_catalog, and Kalam information_schema relations
│   ├── kalamdb-system/
│   │   └── src/
│   │       ├── providers/
│   │       │   ├── keyed_provider.rs                # NEW: KeyedTableProvider<K, V>. PK equality = store.get. Scans use build_record_batch!. No MVCC, overlay, or Row round-trip. Direct typed CRUD stays on IndexedEntityStore
│   │       │   └── catalog/                         # SLIMMED: Delete 11 boilerplate *_provider.rs files and scan.rs; CatalogStores expose KeyedTableProvider. Domain get/put/index methods stay
│   │       └── macros.rs                            # KEEP build_record_batch! / array_builder! / append_value! / finish_array!. DELETE impl_indexed_system_table_provider!, impl_simple_system_table_provider!, impl_system_table_provider_metadata!
│   ├── kalamdb-views/
│   │   └── src/
│   │       ├── catalog_provider.rs                  # NEW: ComputedCatalogProvider shared by virtual system.*, pg_catalog.*, and Kalam information_schema extensions
│   │       ├── system/                              # SLIMMED: row structs move to #[table]; delete hand-built ColumnDefinition lists
│   │       ├── pg_catalog/                          # SLIMMED: keep projection logic; delete per-view Field::new schema() copies
│   │       └── information_schema/                  # KEEP DataFusion stock provider; Kalam extensions use ComputedCatalogProvider + #[table]
│   ├── kalamdb-tables/
│   │   └── src/
│   │       ├── provider/                            # NEW: Single KalamTableProvider implementing DataFusion TableProvider + SourceProvider for User/Shared/Stream
│   │       ├── versioned/                           # EXPANDED: VersionedTableEngine, VersionedIndexedStore, VersionedRowCodec, TargetResolutionCache, TableMutationAccumulator, columnar_hot_scan, covering_aggregate, equality_update (for BOTH User & Shared)
│   │       ├── stream_tables/                       # SLIMMED: StreamTableEngine + StreamTableStore (append-log, TTL, hard-delete only; provider boilerplate deleted)
│   │       ├── shared_tables/                       # SLIMMED: SharedTableAuthorization (FORCE RLS policy binding); duplicate provider/store/pk_index deleted
│   │       ├── user_tables/                         # SLIMMED/COMPAT: Thin compatibility type aliases if needed; duplicate provider/store/pk_index deleted
│   │       └── utils/                               # Shared cold_pk, parquet, vector_staging, core; base/provider.rs, base/deferred.rs, unified_dml/append.rs, and topics/ removed
│   ├── kalamdb-publisher/
│   │   ├── Cargo.toml                               # Remove kalamdb-tables dependency
│   │   └── src/storage/                             # MOVED from kalamdb-tables/src/topics/: TopicMessageStore, TopicMessage, topic_message_schema
│   ├── kalamdb-flush/
│   │   └── src/
│   │       ├── service.rs                           # Lock-free/short-lock mark_pending_write & reclaimable flush_scope_locks
│   │       ├── flush/
│   │       │   ├── versioned.rs                     # NEW: Unified VersionedTableFlushJob replacing flush/users.rs & flush/shared.rs
│   │       │   └── scope_writer.rs                  # Short-hold publication lock (Parquet I/O & hooks outside lock) + streaming batches
│   │       └── compaction/small_segment.rs          # Snapshot watermark awareness during compaction
│   ├── kalamdb-transactions/
│   │   └── src/
│   │       ├── write_set.rs                         # Commit-fold state machine (9 transitions + PK rename) & savepoint preservation
│   │       ├── overlay.rs                           # Typed ScopedTargetKey & accurate buffer_bytes accounting
│   │       └── overlay_exec.rs                      # Columnar RecordBatch overlay merge & minimal column projection
│   ├── kalamdb-core/
│   │   └── src/
│   │       ├── app_context.rs                       # Typed job manager capability (replacing raw Any downcast)
│   │       ├── schema_registry/                     # Store Arc<KalamTableProvider> directly on CachedTableData (zero Any downcasts)
│   │       ├── applier/executor/dml.rs              # Unified versioned PreparedCommitBatch & single StorageBackend::batch commit
│   │       ├── transactions/
│   │       │   ├── commit_sequence.rs               # Active snapshot pin refcounting & min_pinned_version query
│   │       │   └── coordinator.rs                   # Domain/limit preflight & RAII snapshot pin lifecycle
│   │       └── sql/executor/sql_executor/
│   │           ├── execute.rs                       # Complete PlanCacheKey (including postgres_star) & singleflight integration
│   │           ├── literal_dml.rs                   # Use unified KalamTableProvider without User/Shared downcasts
│   │           ├── equality_update.rs               # Support both User & Shared via VersionedTableEngine
│   │           ├── covering_aggregate.rs            # Support both User & Shared via VersionedTableEngine
│   │           ├── returning.rs                     # AST-based RETURNING projection
│   │           └── transactions.rs                  # Delete manual string-split extract_select_from_table_id; use parsed AST
│   ├── kalamdb-dialect/
│   │   └── src/parser/dml/insert_shape.rs           # returning_describe_statement AST constructor (no string re-parse)
│   ├── kalamdb-postgres-wire/
│   │   └── src/statement.rs                         # Use returning_describe_statement AST directly
│   ├── kalamdb-plan-cache/
│   │   └── src/lib.rs                               # postgres_star key dimension, singleflight Notify/LeaderGuard, table-scoped invalidation, recursive byte estimation
│   ├── kalamdb-transfer/
│   │   ├── Cargo.toml                               # Remove kalamdb-core dependency
│   │   └── src/                                     # TransferError & streaming ZIP archive reader/writer
│   ├── kalamdb-jobs/
│   │   └── src/executors/
│   │       ├── flush.rs                             # Use unified VersionedTableFlushJob & KalamTableProvider without downcasts
│   │       ├── table_transfer.rs                    # Bounded streaming export/import on spawn_blocking
│   │       └── user_export.rs                       # Bounded streaming user export on spawn_blocking
│   └── kalamdb-serialization/
│       └── src/object/envelope.rs                   # Durable cluster compatibility floor check for Row V2 writer activation
└── docs/architecture/                               # Updated transaction, storage, and dependency boundary documentation
```

**Structure Decision**: Start unification at the DataFusion `TableProvider` surface (`KalamTableProvider` in `kalamdb-tables` for user/shared/stream; `KeyedTableProvider` in `kalamdb-system` for persisted system stores; `ComputedCatalogProvider` in `kalamdb-views` for `pg_catalog`, virtual `system.*`, and Kalam `information_schema`). Generate every catalog schema from `#[table]`. Move generic DataFusion DML/scan plan helpers into `kalamdb-datafusion-sources`, move misplaced topic persistence from `kalamdb-tables/src/topics/` into `kalamdb-publisher`, unify `User` and `Shared` stores/codecs/indexes/flush into `VersionedTableEngine` + `VersionedTableFlushJob`, and sever `kalamdb-transfer -> kalamdb-core`. Do not put system catalog scans on `VersionedTableEngine`.

---

## Phased Implementation Order & Architecture Plan

Implementation MUST proceed in strict dependency order per `spec.md` Assumptions:

### Phase A — Characterization, Crate Boundary Hygiene & Snapshot Retention Foundation (P0: Stories 1 & 2)
1. **Crate Boundary Cleanup (`FR-033`, `FR-046`)**:
   - Move `kalamdb-tables/src/topics/` (`TopicMessageStore`, `TopicMessage`, `topic_message_schema`) into `kalamdb-publisher` and remove `kalamdb-tables` from `kalamdb-publisher/Cargo.toml`.
   - Remove `kalamdb-core` from `kalamdb-transfer/Cargo.toml` and introduce `TransferError` in `kalamdb-transfer`.
   - Move generic DataFusion DML plan helpers (`rows_affected_plan`, `dml_scan_projection`, `projected_schema`, `PreparedUpdateAssignments`, `collect_input_rows`) into `kalamdb-datafusion-sources`.
2. **Snapshot Pin Watermarks (`CommitSequenceTracker`)**:
   - Track active snapshot pins per `GroupId` with RAII release on transaction commit, rollback, or timeout; expose `min_pinned_version(group_id) -> Option<VersionId>`.
3. **Snapshot-Aware Hot Replacement (`IndexedEntityStore::push_replace`)**:
   - Retain prior hot row and PK index entry when `min_pinned_version(group_id)` shows an active reader pinned at `old_version <= snapshot < new_version`; otherwise replace in-place in the `WriteBatch`.
4. **Baseline Characterization & Stage Metrics**:
   - Record baseline `StorageBackend::batch` submission counts, PK target loads, production LOC across the consolidation scope, and `dev`-profile runtimes (in seconds) for 25k insert/update/delete and dirty-read workloads.
   - Record the SC-015 system-catalog baseline in the same pass: direct user/job/namespace/type lookups and one `pg_catalog` + `information_schema` introspection query. Later catalog edits are compared to this baseline.

### Phase B — DataFusion-Provider-First Unification & Versioned/System Store Consolidation (P0: Story 2, FR-001–FR-004, FR-038–FR-047)
1. **System catalog fast path, schema macro, and computed catalog provider (SC-015)**:
   - Record the SC-015 baseline before editing catalog providers: direct `system.users` / `system.jobs` / namespace / type point and index lookups, plus one `pg_catalog` + `information_schema` introspection query. Candidate median and p99 must be ≤ baseline.
   - Extend `#[table]` in `kalamdb-macros` so persisted system models, virtual `system.*` row structs, `pg_catalog` row structs, and Kalam `information_schema` extensions all get `definition()`, a `OnceLock` Arrow schema from `definition().to_arrow_schema()`, the primary-key column, and ordinal column order. Do not add a second schema language.
   - Keep `build_record_batch!` in `kalamdb-system/src/macros.rs`. Delete only `impl_indexed_system_table_provider!`, `impl_simple_system_table_provider!`, `impl_system_table_provider_metadata!`, `SystemTableScan`, and `SimpleSystemTableScan`.
   - Implement `KeyedTableProvider<K, V>` so SQL primary-key equality calls `store.get`, secondary predicates use the existing index iterator, and full scans call `build_record_batch!`. Reject SQL DML. Leave domain methods (`get_user_by_id`, job index scans, catalog `get`/`put`) on `IndexedEntityStore` with their current caches. Do not route them through DataFusion, `VersionedTableEngine`, overlay, or a `Row` conversion.
   - Implement `ComputedCatalogProvider` in `kalamdb-views` for virtual `system.*`, `pg_catalog.*`, and Kalam `information_schema` extensions. Keep each relation's projection function. Delete hand-written `Field::new` schemas and hand-built `ColumnDefinition` lists once `#[table]` emits them. Leave DataFusion's stock `information_schema` provider in place for relations Kalam does not own.
   - Delete the 11 boilerplate `kalamdb-system/src/providers/catalog/*_provider.rs` files and `catalog/scan.rs` after `CatalogStores` construct `KeyedTableProvider` from `Model::definition()`.
2. **Unified `ScopedRowKey`, `VersionedTableRow`, and `VersionedRowCodec`**:
   - Implement `ScopedRowKey` (`User(UserTableRowId)` | `Shared(SharedTableRowId)`) and `VersionedTableRow` in `kalamdb-commons` with bit-for-bit identical `storekey` and KOBJ encodings.
   - Replace `UserRowCodec`/`SharedRowCodec` and `UserTablePkIndex`/`SharedTablePkIndex` with `VersionedRowCodec` and `PrefixIndex<ScopedRowKey, VersionedTableRow>`.
3. **Unified `VersionedTableEngine` & `StreamTableEngine` (`kalamdb-tables`)**:
   - Consolidate hot+cold MVCC scans, `count_resolved_rows_async`, PK lookups, ordinal payload decoding, vector staging, `columnar_hot_scan`, `sum_hot_covering_integers`, `plan_literal_equality_updates`, and `TableMutationAccumulator` preparation into `VersionedTableEngine` (serving both `User` and `Shared` tables).
   - Keep `StreamTableEngine` focused purely on `StreamTableStore` append, TTL windowing, and hard-delete.
4. **Single DataFusion `KalamTableProvider` (`kalamdb-tables/src/provider/`)**:
   - Implement `datafusion::datasource::TableProvider` (`scan`, `insert_into`, `delete_from`, `update`, `supports_filters_pushdown`, `statistics`) and `SourceProvider` **once** on `KalamTableProvider` over `TableStorageEngine` (`Versioned` vs `Stream`).
   - Store `Arc<KalamTableProvider>` directly on `CachedTableData` in `SchemaRegistry` and eliminate all 35+ `.downcast_ref::<UserTableProvider | SharedTableProvider | StreamTableProvider>()` sites across `kalamdb-core`, `kalamdb-jobs`, `kalamdb-pg-bridge`, and `kalamdb-dba`.
   - Delete superseded `UserTableProvider`, `SharedTableProvider`, `StreamTableProvider` files, `BaseTableProvider<K, V>`, `DeferredMvccScanProvider<K, V>`, and `utils/unified_dml/append.rs`.
5. **Unified `VersionedTableFlushJob` (`kalamdb-flush`)**:
   - Replace `UserTableFlushJob` (`flush/users.rs`) and `SharedTableFlushJob` (`flush/shared.rs`) with one scope-aware `VersionedTableFlushJob`.

### Phase C — Group-Wide Atomic Transaction Apply (P0: Story 1, FR-005–FR-012)
1. **Unified Pure Preparation in `DmlExecutor` (`kalamdb-core/src/applier/executor/dml.rs`)**:
   - Replace the parallel user and shared transaction apply/prevalidate/cleanup methods in `dml.rs` with one domain-scoped pipeline that prepares all touched tables into one `PreparedCommitBatch` (`Vec<Operation>` + `PostCommitEffectPlan`) via `VersionedTableEngine`.
   - Validate all constraints, `FILE` refs, RLS `WITH CHECK` policies, uniqueness, and version/byte limits before any storage write.
2. **Single `StorageBackend::batch` Commit & Post-Persist Ordering**:
   - Submit all main-row, PK-index, secondary-index, and pending-manifest operations in one atomic `StorageBackend::batch` call (without relying on `task_local!` across `spawn_blocking`).
   - Execute frontier advancement, in-memory manifest updates, vector staging, topic/live notifications, and file deletions strictly after `StorageBackend::batch` succeeds.
   - Verify uncoalesced 25,000-row atomic equivalence and fault-injection rollback before enabling commit folding.

### Phase D — Commit-Time Write-Set Folding, Target Reuse & Columnar Overlay (P1: Story 3, FR-013–FR-017)
1. **Commit-Fold State Machine (`kalamdb-transactions/src/write_set.rs`)**:
   - Implement the 9-transition table + PK rename expansion keyed by `(TableId, TableScope, PkBucketKey)` while preserving `ordered_mutations` for savepoint rollback and statement affected counts.
2. **Authoritative Target Resolution Cache (`kalamdb-tables/src/versioned/target.rs`)**:
   - Batch-load each distinct `ScopedTargetKey` at most once per commit and verify expected version bounds (`FR-011`, `FR-015`).
3. **Columnar `TransactionOverlayExec` (`kalamdb-transactions/src/overlay_exec.rs`)**:
   - Push down minimal required column projection in `KalamTableProvider::scan` and process untouched/filtered batches in Arrow columnar form without whole-table `Vec<Row>` conversion.

### Phase E — Secondary Index Tombstone Policy & History/Live Sync Contracts (P1: Story 4, FR-018–FR-021)
1. **Explicit `IndexRole` & Tombstone Omission (`kalamdb-store/src/indexed_store.rs`)**:
   - Keep tombstones in `IndexRole::PrimaryKey` and omit new tombstones from `IndexRole::SecondaryScalar`.
   - Ensure `_deleted = true` and `since_seq` queries use complete authoritative paths and online index rebuild publishes generations atomically.
2. **Complete-Event vs Latest-State Contract**:
   - Preserve complete-event delivery / explicit gap signaling on expired cursors.

### Phase F — Non-Blocking Flush Publication, Bounded Transfers & Contextual Plan Cache (P1: Stories 5 & 6, FR-022–FR-029)
1. **Short-Lock Flush & Scope Lock Reclamation (`kalamdb-flush`)**:
   - Decouple `mark_pending_write` from bulk flush locks; move Parquet write/rename and scope hooks outside `flush_scope_lock`; reclaim idle `flush_scope_locks` entries.
2. **Streaming Export/Import (`kalamdb-transfer`, `kalamdb-jobs`)**:
   - Stream ZIP archives to/from disk on `spawn_blocking` within bounded memory.
3. **Contextual `PlanCacheKey`, Singleflight Misses & AST Semantics (`kalamdb-plan-cache`, `kalamdb-dialect`, `kalamdb-core`, `kalamdb-postgres-wire`)**:
   - Add `postgres_star` to `PlanCacheKey`; add `LeaderGuard` + `Notify` singleflight coordination; replace `returning_describe_sql` string round-trip and `extract_select_from_table_id` manual string scanning with AST operations.

### Phase G — Row V2 Compatibility Barrier & Typed Job Orchestration (P2: Story 7, FR-030–FR-037)
1. **Durable Row V2 Compatibility Gate (`kalamdb-serialization`, `kalamdb-core`)**:
   - Gate `set_row_v2_writer_enabled` on cluster-wide eligible-replica compatibility; verify mixed V1/V2 reads and safe writer disable.
2. **Typed Job Manager in `AppContext` (`kalamdb-core`, `kalamdb-jobs`)**:
   - Replace raw `Arc<dyn Any>` job manager access with a typed trait capability and startup executor registration check.
3. **Final Consolidation Accounting, Full Regression Sweep & Architecture Docs**:
   - Verify positive net LOC reduction and zero concrete provider downcasts (SC-005), run full `cargo nextest run` and CLI smoke suites (SC-012), and update `docs/architecture/`.

## Complexity Tracking

> **No Constitution violations require justification.** The plan removes two crate dependency edges (`kalamdb-transfer -> kalamdb-core`, `kalamdb-publisher -> kalamdb-tables`), introduces zero new external dependencies, eliminates 35+ runtime `Any` downcasts, and removes ~3,500–4,500+ lines of duplicated code.
