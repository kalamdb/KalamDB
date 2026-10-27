# Phase 0 Research: Unified Table Processing and Atomic Transaction Optimization

**Feature**: `039-unified-table-transactions`  
**Branch**: `feat/pg-dialect` (PR [#402](https://github.com/kalamdb/KalamDB/pull/402))  
**Date**: 2026-10-27  
**Spec**: [spec.md](spec.md) | **Source Review**: [source-review.md](source-review.md)

---

## 1. DataFusion-Provider-First Unification & Crate Boundary Realignment (FR-001–FR-004, FR-038–FR-046)

### Root Cause of Existing Duplication
A full audit of `kalamdb-tables`, `kalamdb-system`, `kalamdb-datafusion-sources`, `kalamdb-flush`, `kalamdb-core`, `kalamdb-jobs`, `kalamdb-pg-bridge`, and `kalamdb-dba` revealed why ~6,000+ lines of duplicate code exist today:

1. **Four Parallel DataFusion `TableProvider` Worlds (`User`, `Shared`, `Stream`, and `System`)**:
   - `UserTableProvider` (~2,792 LOC), `SharedTableProvider` (~3,530 LOC), and `StreamTableProvider` (~862 LOC) in `kalamdb-tables` each implement `datafusion::datasource::TableProvider` (`scan`, `insert_into`, `delete_from`, `update`, `supports_filters_pushdown`), `BaseTableProvider<K, V>`, `KalamTableProvider`, and `SourceProvider` independently.
   - Meanwhile, `kalamdb-system` implements **22 separate persisted `*TableProvider` structs** across `kalamdb-system/src/providers/` (~3,200+ LOC) using two more custom traits (`SystemTableScan<K, V>` and `SimpleSystemTableScan<K, V>` in `providers/base.rs`, 1,005 LOC), macro-generated `TableProvider` impls (`macros.rs`, 435 LOC), 11 nearly identical `providers/catalog/*_provider.rs` wrapper files (~950 LOC), and an extra `SecuredSystemTableProvider` wrapper in `kalamdb-session-datafusion`!
   - Yet persisted `system.*` tables have the **exact same foundation** as `Shared` tables:
     - Every persisted system table has a `TableDefinition` (`User::definition()`, `Job::definition()`, `CatalogType::definition()`, etc.).
     - Every persisted system table is backed by `IndexedEntityStore<K, SystemTableRow>` (or `IndexedEntityStore<K, Model>` mapped via `model_to_system_row` into `Row` / Arrow `RecordBatch`) with primary key and secondary indexes (`IndexDefinition`).
     - The only semantic differences between a persisted `System` table and a `Shared` table are:
       1. **Keying & MVCC**: `System` tables are keyed directly by their domain primary key `K` (`UserId`, `JobId`, `TypeId`, etc.) and mutated in-place without `_version`/`_deleted` MVCC columns or cold Parquet flushing.
       2. **Mutation Surface**: `System` tables are **read-only from SQL DML** (`TableProvider::{insert_into, update, delete_from}` and explicit SQL transactions are rejected) and mutated via **direct typed Rust APIs** on their `IndexedEntityStore<K, V>` / domain service (bypassing DataFusion).
       3. **Access Control**: `System` tables enforce `check_system_table_access` in `scan()` (admin/system roles only) instead of `check_shared_table_access` + FORCE RLS.
2. **Generic `<K, V>` Trait Leakage (`BaseTableProvider<K, V>` & `DeferredMvccScanProvider<K, V>`)**:
   - Because `BaseTableProvider<K, V>` is generic over `<UserTableRowId, UserTableRow>` vs `<SharedTableRowId, SharedTableRow>` vs `<StreamTableRowId, StreamTableRow>`, it is not dyn-compatible.
   - As a result, `SchemaRegistry` stores `Arc<dyn TableProvider + Send + Sync>` and **35+ call sites across 6 crates** (`DmlExecutor`, `SqlExecutor`, `FlushExecutor`, `TableExportExecutor`, `UserExportExecutor`, `StreamEvictionExecutor`, `OperationService`, `LiveAuthorizationBinderAdapter`, `SharedTableRepository`) downcast `Any` to `UserTableProvider`, `SharedTableProvider`, or `StreamTableProvider` and duplicate the caller logic!
3. **Intra-Provider Duplication (`dml.rs` vs `sql.rs`)**:
   - Inside both `UserTableProvider` and `SharedTableProvider`, `dml.rs` implements `update_by_pk_value_deferred_with_hot` and `delete_by_pk_value_deferred_with_hot`, while `sql.rs` contains a second ~250-line copy (`update_by_pk_value_with_version` and `delete_by_pk_value_with_version`)!
4. **Unnecessary Split Between `User` and `Shared` Versioned Storage, Indexes, Codecs, Vectors, and Flush**:
   - On disk in RocksDB, KOBJ row values for `User` and `Shared` tables are **100% identical** (`encode_row_body` and `decode_row_body` in `kalamdb-serialization` do not store `user_id` or `_version` in the value bytes; they are reconstructed from the key).
   - `PrefixIndex` in `kalamdb-store` **already** supports `user_scoped: bool` and `Option<&UserId>`.
   - Yet the codebase maintains separate `UserTableRowId`/`SharedTableRowId`, `UserTableRow`/`SharedTableRow`, `UserRowCodec`/`SharedRowCodec`, `UserTablePkIndex`/`SharedTablePkIndex`, `UserTableIndexedStore`/`SharedTableIndexedStore`, `UserVectorHotOpId`/`SharedVectorHotOpId`, and `UserTableFlushJob`/`SharedTableFlushJob`!
   - Because of this split, `columnar_hot_scan`, `sum_hot_covering_integers`, and `plan_literal_equality_updates` were only implemented on `SharedTableProvider` and are missing on `UserTableProvider`.

### Decision: DataFusion-Provider-First Architecture

1. **One Unified DataFusion Provider (`KalamTableProvider` struct in `kalamdb-tables`)**:
   - Replace the three concrete provider structs (`UserTableProvider`, `SharedTableProvider`, `StreamTableProvider`) and retire the redundant traits (`BaseTableProvider<K, V>`, `DeferredMvccScanProvider<K, V>`, and the old 4-method `KalamTableProvider` trait) in favor of **one concrete `KalamTableProvider` struct** that implements `datafusion::datasource::TableProvider` and `kalamdb_datafusion_sources::SourceProvider` once for all three table kinds (`User`, `Shared`, `Stream`).
   - `KalamTableProvider` holds:
     - `core: Arc<TableProviderCore>` (shared metadata, `TableId`, memoized Arrow `SchemaRef`, `StorageSchema`, column defaults, `non_null_columns`, `TableServices`, and `TableCapabilityProfile`).
     - `engine: TableStorageEngine`:
       - `TableStorageEngine::Versioned(VersionedTableEngine)` — backs **both** `User` and `Shared` tables!
       - `TableStorageEngine::Stream(StreamTableEngine)` — backs `Stream` tables (`StreamTableStore` + `ttl_seconds`).
   - **DataFusion `TableProvider` methods implemented ONCE on `KalamTableProvider`**:
     - `scan()`: runs unified access & session context extraction (`resolve_scan_scope_and_auth`), `validate_transaction_table_access`, and `ensure_leader_read`; builds `ScanDescriptor`, physical residual filter, and `output_projection` remapping once via `kalamdb-datafusion-sources`; returns `DeferredBatchExec` (wrapped in `TransactionOverlayExec` with minimal column projection when an active transaction overlay exists for `table_id`).
     - `insert_into()`: validates `InsertOp::Append`, write access, and write route; collects input rows; validates foreign keys and RLS `WITH CHECK` policies (when `rls_mode == ForcePolicyCheck`); stages into active transaction (`Versioned`) or executes batch insert (`Versioned` / `Stream`) and returns `rows_affected_plan`.
     - `delete_from()`: validates write access and route; extracts target PKs via `pk_membership_literals` fast path (when RLS permits) or `collect_matching_rows_with_projection`; deduplicates PKs; stages into active transaction (with `pk_claims` + `committed_primary_key_is_live` recheck) or executes batch delete (soft-delete tombstones for `Versioned`, hard-delete for `Stream`).
     - `update()`: checks `capabilities.supports_update()` (returns `Plan("UPDATE not supported for STREAM tables")` for `Stream`); validates write access, route, and assignments; computes minimal DML scan projection (`dml_scan_projection`); collects matching rows and evaluates assignments in vectorized batches via `PreparedUpdateAssignments`; enforces RLS `WITH CHECK` on updated rows; handles PK relocation (`Delete(old_pk)` + `Insert(new_pk)`) and no-op suppression; stages into active transaction or executes batch update.
2. **Unified `ScopedRowKey`, `VersionedTableRow`, `VersionedRowCodec`, and `VersionedIndexedStore` (`User` + `Shared`)**:
   - `ScopedRowKey` (`kalamdb-commons::ids`): enum `User(UserTableRowId)` (`(user_id, version)` storekey bytes) | `Shared(SharedTableRowId)` (`version` storekey bytes). Preserves **bit-for-bit identical** RocksDB keys on disk while unifying the Rust key type!
   - `VersionedTableRow` (`kalamdb-commons::models::rows`): `{ scope: Option<UserId>, _version: VersionId, _deleted: bool, fields: Row }`.
   - `VersionedRowCodec`: one `EntityCodec<ScopedRowKey, VersionedTableRow>` replacing `UserRowCodec` and `SharedRowCodec`.
   - Delete `UserTablePkIndex` and `SharedTablePkIndex` wrapper structs — use `kalamdb_store::PrefixIndex<ScopedRowKey, VersionedTableRow>` directly.
   - `VersionedTableEngine` owns hot+cold MVCC scans, `count_resolved_rows_async`, `columnar_hot_scan`, `sum_hot_covering_integers`, `plan_literal_equality_updates`, PK lookups, ordinal payload decoding, vector staging, and `TableMutationAccumulator` preparation **once** for both `User` and `Shared` tables!
3. **Unified `VersionedTableFlushJob` (`kalamdb-flush`)**:
   - Collapse `UserTableFlushJob` (`flush/users.rs`, 360 LOC) and `SharedTableFlushJob` (`flush/shared.rs`, 284 LOC) into a single `VersionedTableFlushJob` over `VersionedIndexedStore` that flushes per `scope: Option<UserId>` (`Some(user_id)` partitions for User tables, `None` for Shared tables).
4. **System catalog tables stay on the fast path (`KeyedTableProvider` + `ComputedCatalogProvider` + `#[table]`)**:
   - Because `kalamdb-filestore` and `kalamdb-vector` depend on `kalamdb-system`, `kalamdb-system` must not depend on `kalamdb-tables`. `kalamdb-views` must not depend on `kalamdb-tables` either. Both share `kalamdb-datafusion-sources`, `kalamdb-store` (persisted side only), and `kalamdb-session-datafusion`.
   - **Persisted system tables** (`users`, `jobs`, `job_nodes`, `namespaces`, `storages`, `audit_log`, `topics`, `topic_offsets`, `migrations`, `table_policies`, manifest, and the 11 `CatalogStores` tables) use one `KeyedTableProvider<K, V>` for SQL `SELECT` only:
     - Runtime callers keep `IndexedEntityStore::get` / `scan_index` / `put` and the caches already in front of them (privileged-role cache, schema registry). Those calls do not enter DataFusion.
     - SQL primary-key equality is `store.get`, then a one-row `build_record_batch!`. It is not a scan that filters afterward.
     - Secondary predicates use the existing index iterator. Full scans use `build_record_batch!` in one pass from the typed value.
     - No `VersionedTableEngine`, no `select_latest_versions`, no `TransactionOverlayExec`, no Parquet, no `V -> Row -> RecordBatch` conversion. That conversion would be a performance downgrade versus today's single-pass macro.
     - SQL DML and explicit SQL transactions stay rejected.
   - **Computed catalog tables** are the same system-catalog family, not user tables:
     - Virtual `system.*` views (`sessions`, `stats`, `live`, `settings`, `cluster`, and the other views registered by `SystemSchemaProvider`).
     - `pg_catalog.*` (`PgCatalogSchemaProvider` / `PgCatalogView`, including `pg_class`, `pg_attribute`, `pg_type`, `pg_namespace`, and the empty compatibility views).
     - Kalam-owned `information_schema` extensions. DataFusion's stock `InformationSchemaProvider` stays for relations Kalam does not replace.
     - One `ComputedCatalogProvider` owns access checks, projection, limit, and the macro-generated schema. Each relation keeps its specialized in-memory `compute` function. Nothing here is written to RocksDB or flushed.
   - **Schema macros are kept and become the schema source**:
     - `kalamdb_macros::table` already emits `definition() -> TableDefinition` for persisted models (`User`, `Job`, catalog types). Extend it to emit `arrow_schema()` (`OnceLock` over `definition().to_arrow_schema()`), the primary-key column, and ordinal order.
     - Move virtual `system.*` hand-built `ColumnDefinition` lists (`sessions.rs` and siblings) and `pg_catalog` hand-written `Field::new` schemas (`class.rs` and siblings) onto `#[table(namespace = "system" | "pg_catalog" | "information_schema")]`.
     - Keep `build_record_batch!`, `array_builder!`, `append_value!`, and `finish_array!`. They are the hot encoder.
     - Delete `impl_indexed_system_table_provider!`, `impl_simple_system_table_provider!`, `impl_system_table_provider_metadata!`, `SystemTableScan`, `SimpleSystemTableScan`, the 11 `catalog/*_provider.rs` wrappers, and `catalog/scan.rs`. Those copy `TableProvider`; they do not define schema.
   - **Performance gate (SC-015)**: record baseline before the edit. Direct point/index lookups and one wire introspection query must have median and p99 ≤ baseline across five matched `dev` repeats. Any regression blocks the change. The user/shared 5% allowance does not apply.
5. **Crate Boundary Realignment (FR-033, FR-046)**:
   - **`kalamdb-datafusion-sources`**: Absorb generic DataFusion DML/scan plan helpers (`rows_affected_plan`, `dml_scan_projection`, `projected_schema`, `PreparedUpdateAssignments`, `collect_input_rows`) from `kalamdb-tables/src/utils/datafusion_dml.rs` so `kalamdb-datafusion-sources` is the single home for DataFusion `TableProvider`/`ExecutionPlan` primitives shared by `kalamdb-system` and `kalamdb-tables`.
   - **`kalamdb-publisher` vs `kalamdb-tables`**: Move `kalamdb-tables/src/topics/` (`TopicMessageStore`, `TopicMessage`, `topic_message_schema`) into `kalamdb-publisher` (its true owner per `AGENTS.md`) and drop the `kalamdb-tables` dependency from `kalamdb-publisher/Cargo.toml`.
   - **`kalamdb-transfer` vs `kalamdb-core`**: Remove `kalamdb-core` from `kalamdb-transfer/Cargo.toml` and introduce `TransferError` in `kalamdb-transfer`.

### Rationale
- Starting unification at the DataFusion `TableProvider` surface eliminates the 3-way split at the root instead of trying to share small helpers underneath three diverging structs.
- Eliminates all 35+ `.downcast_ref::<UserTableProvider | SharedTableProvider | StreamTableProvider>()` sites across the workspace.
- Automatically gives `User` tables the high-performance hot-storage fast paths (`columnar_hot_scan`, `sum_hot_covering_integers`, `plan_literal_equality_updates`) that previously only existed on `SharedTableProvider`.
- Shrinks the maintained codebase by thousands of lines while improving compile-time crate decoupling.
- Keeps system catalog runtime on the current fast path: direct store gets for persisted tables, in-memory projections for `pg_catalog` and virtual views, and `#[table]` as the single schema source. Putting those scans through `VersionedTableEngine` or a `Row` round-trip would miss SC-015.

---

## 2. Mandatory Duplication Inventory & Deletion Map (FR-044, FR-045, FR-046, SC-005)

### Verified Baseline Inventory (Current `feat/pg-dialect` Tree)

| Area / Responsibility | Current Duplicated Files & Line Counts | Duplicated Functions / Types | Target Owner Crate & Module | Deletion / Consolidation Target |
| :--- | :--- | :--- | :--- | :--- |
| **DataFusion `TableProvider` & SQL Entry Points** | `kalamdb-tables/src/user_tables/provider/{mod,sql,scan}.rs` (1,068 LOC)<br>`kalamdb-tables/src/shared_tables/provider/{mod,sql,scan}.rs` (1,186 LOC)<br>`kalamdb-tables/src/stream_tables/provider/{mod,sql,scan_source,operations}.rs` (862 LOC)<br>`kalamdb-tables/src/utils/base/{provider,deferred}.rs` (1,829 LOC) | 3 `TableProvider` impls (`scan`, `insert_into`, `delete_from`, `update`), 3 `KalamTableProvider` impls, 2 `DeferredMvccScanProvider` impls, `BaseTableProvider<K, V>`, plus intra-file duplicate `update_by_pk_value_with_version` & `delete_by_pk_value_with_version` in `sql.rs` | `kalamdb-tables/src/provider/` (`KalamTableProvider`, `TableScanSource`) + `kalamdb-datafusion-sources` | Delete `user_tables/provider/{sql,scan}.rs`, `shared_tables/provider/{sql,scan}.rs`, `stream_tables/provider/{sql,scan_source}.rs`, and retire `BaseTableProvider<K, V>` & `DeferredMvccScanProvider<K, V>`. |
| **Versioned DML, Scan & Target Resolution** | `kalamdb-tables/src/user_tables/provider/dml.rs` (900 LOC)<br>`kalamdb-tables/src/shared_tables/provider/dml.rs` (873 LOC) | `scan_with_version_resolution_to_kvs_result_async`, `count_resolved_rows_async`, `insert_deferred*`, `insert_batch*`, `update_by_pk_value_deferred*`, `delete_by_pk_value_deferred*`, `delete_batch_by_pk_values_deferred`, `update_batch_by_pk_values_deferred` | `kalamdb-tables/src/versioned/engine.rs` | Delete both `dml.rs` files; replace with single `VersionedTableEngine` implementation. |
| **Versioned Storage, PK Index, Codec, Ordinal Decode & Fast Paths** | `kalamdb-tables/src/user_tables/provider/storage.rs` (951 LOC)<br>`kalamdb-tables/src/shared_tables/provider/{storage,pk_maintenance,columnar_scan,covering_aggregate,equality_update}.rs` (1,580 LOC)<br>`kalamdb-tables/src/user_tables/{user_table_store,pk_index}.rs` (634 LOC)<br>`kalamdb-tables/src/shared_tables/{shared_table_store,pk_index}.rs` (846 LOC)<br>`kalamdb-tables/src/row_codec.rs` (155 LOC) | `UserRowCodec` vs `SharedRowCodec`, `UserTablePkIndex` vs `SharedTablePkIndex`, `stage_vector_*`, `validate_insert_batch_rows`, `persist_insert_batch_rows`, `insert_encoded_fields_with_versions`, `load_mutation_target`, `insert_predecessor`, `scan_hot_pk_insert`, shared-only `columnar_hot_scan` / `covering_aggregate` / `equality_update` | `kalamdb-tables/src/versioned/{storage,fast_paths,codec}.rs` + `kalamdb-store::PrefixIndex` | Delete `user_tables/pk_index.rs`, `shared_tables/pk_index.rs`, duplicate `storage.rs` files, and `UserRowCodec`/`SharedRowCodec` split; unify into `VersionedTableEngine` + `VersionedRowCodec` so User & Shared share all fast paths. |
| **Legacy JSON `append_version` Module** | `kalamdb-tables/src/utils/unified_dml/append.rs` (193 LOC) | `append_version`, `append_version_sync_with_deps` | Superseded by `VersionedTableEngine` | Delete `append.rs`. |
| **Misplaced Topic Persistence in `kalamdb-tables`** | `kalamdb-tables/src/topics/{mod,topic_message_models,topic_message_schema,topic_message_store}.rs` (~650 LOC) | `TopicMessageStore`, `TopicMessage`, `topic_message_schema` | `kalamdb-publisher/src/storage/` | Move topic store/models to `kalamdb-publisher`; remove `kalamdb-tables` dependency from `kalamdb-publisher/Cargo.toml`. |
| **User vs Shared Flush Jobs in `kalamdb-flush` & `kalamdb-jobs`** | `kalamdb-flush/src/flush/users.rs` (360 LOC)<br>`kalamdb-flush/src/flush/shared.rs` (284 LOC)<br>`kalamdb-jobs/src/executors/flush.rs` (`FlushTarget::User` vs `FlushTarget::Shared`) | Batched cursor scan, version dedup, tombstone cold mask, `delete_flushed_keys` vs `delete_flushed_rows` | `kalamdb-flush/src/flush/versioned.rs` (`VersionedTableFlushJob`) | Delete `flush/users.rs` and `flush/shared.rs` in favor of unified `VersionedTableFlushJob`. |
| **Applier DML & Transaction Batch Execution + 35+ Provider Downcasts** | `kalamdb-core/src/applier/executor/dml.rs` (1,852 LOC)<br>`kalamdb-core/src/sql/executor/sql_executor/{literal_dml,equality_update,covering_aggregate}.rs`<br>`kalamdb-pg-bridge/src/service.rs`<br>`kalamdb-dba/src/repository/shared_repository.rs` | Parallel `*_user_*` and `*_shared_*` applier methods, `prevalidate_user/shared_transaction_batch`, `apply_user/shared_transaction_batch_with_versions`, `load_user/shared_row_for_cleanup`, and 35+ `downcast_ref::<UserTableProvider\|SharedTableProvider\|StreamTableProvider>()` sites | `kalamdb-core/src/applier/executor/dml.rs` & `KalamTableProvider` | Delete parallel user/shared applier methods and all concrete provider downcasts; route through `Arc<KalamTableProvider>` and `VersionedTableEngine`. |
| **System catalog providers, duplicated schemas, and provider-impl macros** | `kalamdb-system/src/providers/base.rs` (1,005 LOC)<br>`kalamdb-system/src/macros.rs` provider-impl half<br>`kalamdb-system/src/providers/catalog/{scan,*_provider}.rs` (12 files, ~1,020 LOC)<br>`kalamdb-views/src/pg_catalog/*.rs` hand-written `schema()`<br>`kalamdb-views/src/system/*.rs` hand-built `definition()` | `SystemTableScan`, `SimpleSystemTableScan`, `impl_*_system_table_provider!`, 11 `Catalog*TableProvider` wrappers, per-view `Field::new` schemas, per-view `ColumnDefinition` lists | `KeyedTableProvider` + `ComputedCatalogProvider` + extended `#[table]` | Delete provider-impl macros, catalog wrapper files, and hand-written schemas. Keep `#[table]` and `build_record_batch!`. Do not move runtime `get`/`put` onto DataFusion. |

**Baseline Targeted Production Code**: ~15,000+ lines across the inventoried provider, system-provider, store, flush, and applier files.  
**Consolidation Target**: Eliminate ~4,500–5,500+ lines of duplicated production code. User, shared, and stream share `KalamTableProvider`. Persisted system tables share `KeyedTableProvider` on the direct store. `pg_catalog`, virtual `system.*`, and Kalam `information_schema` share `ComputedCatalogProvider`. System catalog latency meets SC-015. Provider-impl macros and hand-written duplicate schemas are deleted; `#[table]` and `build_record_batch!` stay (SC-005, SC-013, SC-014, SC-015).

---

## 3. Group-Wide Atomic Commit & Pure Preparation (FR-005–FR-008, FR-010–FR-012)

### Decision
Replace per-row / per-table immediate storage writes and side effects during transaction apply with a **two-phase Prepare-then-Commit pipeline** inside the single commit domain:

1. **Phase 1 — Pure Validation & Operation Construction (`PreparedCommitBatch`)**:
   - Validate transaction state, group binding, leadership, and byte/row/version limits (`<= 65,536` version slots per Raft entry per ADR-022; configured `max_transaction_buffer_bytes`).
   - Validate all `FILE` references, schema coercions, `NOT NULL` constraints, foreign keys, RLS `WITH CHECK` policies, and primary-key uniqueness/conflict rules across all touched tables in the commit domain.
   - Build a single `Vec<kalamdb_store::Operation>` containing all main-table `Put`/`Delete` operations, all PK and secondary scalar index `Put`/`Delete` operations (via `IndexedEntityStore::append_replace_ops` / `append_insert_ops`), and all manifest pending-write index operations.
   - Collect deferred side-effect intents (`PostCommitEffectPlan`: vector hot-staging operations, manifest memory-cache updates, file refs to delete, topic publish events, live query notifications) without executing any of them during Phase 1.
   - If any validation or preparation step fails for row $k$ (e.g., row 25,000 of 25,000), Phase 1 returns `Err(...)` and drops `PreparedCommitBatch`. Zero RocksDB operations, zero file deletions, zero vector writes, zero manifest memory mutations, and zero notifications occur.
2. **Phase 2 — Single Atomic Storage Submission & Post-Persist Hooks**:
   - Submit the entire `Vec<Operation>` in **one** `StorageBackend::batch(operations)` call (without relying on `task_local!` `COALESCE_ID` across `spawn_blocking` boundaries).
   - Only after `StorageBackend::batch` returns `Ok(())`:
     1. Advance `CommitSequenceTracker::observe_committed(group_id, log_index)`.
     2. Apply in-memory manifest cache updates.
     3. Apply idempotent vector staging batch operations.
     4. Publish durable topic events and dispatch live query notifications.
     5. Execute best-effort file cleanup for replaced/deleted `FILE` columns.

---

## 4. Snapshot Safety & Hot-Version Retention (FR-009, FR-019, SC-002)

### Decision
Make hot-row replacement (`IndexedEntityStore::push_replace` / `VersionedTableEngine`) **snapshot-aware**:

1. Track active snapshot pins per `GroupId` in `CommitSequenceTracker` (released via RAII guard on transaction commit, rollback, or timeout) and expose `min_pinned_version(group_id) -> Option<VersionId>`.
2. When replacing a hot predecessor `(old_key, old_entity)` with `new_version`:
   - If any active pinned snapshot has `old_version <= snapshot_bound < new_version`, **retain `old_key`** and its PK index entry so repeatable-read snapshot scans and point lookups at `snapshot_bound` continue to resolve `old_version` even before cold flush.
   - When no active snapshot requires `old_version`, delete `old_key` and its superseded index entries in the same `WriteBatch`.
3. Flush and compaction respect `min_pinned_version(group_id)` and retain tombstones masking older cold segments, with full prior row fields preserved on tombstones (`FR-019`).

---

## 5. Commit-Time Write-Set Folding & Authoritative Target Reuse (FR-011, FR-013–FR-015, FR-018, SC-003)

### Decision
1. Keep statement-ordered `ordered_mutations: Vec<StagedMutation>` during the open transaction for exact savepoint rollback (`rollback_to_savepoint`) and statement-level affected counts.
2. At commit preparation, fold mutations per `(TableId, TableScope, PkBucketKey)` according to the 9-row state transition table in `spec.md` (plus PK rename decomposition `Delete(old_pk)` + `Insert(new_pk)`).
3. Resolve each distinct `(TableId, TableScope, PkBucketKey)` at most once via `TargetResolutionCache` (batched hot PK lookup + batched cold check for hot misses) and enforce expected committed version checks (`FR-011` write-conflict detection).

---

## 6. Columnar Transaction Overlay Reads (FR-016, FR-017, SC-010)

### Decision
1. In `KalamTableProvider::scan` (overlay path), project only `caller_projection ∪ filter_columns ∪ {pk_column}` from the base scan instead of `None` (all columns).
2. In `TransactionOverlayExec`, extract `PkBucketKey`s directly from the child `RecordBatch` PK column array; pass disjoint batches through with zero `Row` conversions, apply deletes via Arrow `filter_record_batch` masks, and patch/append updated and inserted rows in columnar form while maintaining accurate `buffer_bytes` accounting.

---

## 7. Secondary Index Tombstone Policy & Online Generation Rebuild (FR-019–FR-021, FR-031)

### Decision
1. Distinguish `IndexRole::PrimaryKey` (retains tombstones so hot PK point lookups and uniqueness checks see deleted keys in $O(1)$ without cold scans) from `IndexRole::SecondaryScalar` (omits new tombstones on delete so range scans and `sum_visible_index_integers` skip dead keys).
2. Queries requesting deleted rows (`_deleted = true` or `since_seq` history scans) bypass `IndexRole::SecondaryScalar` and use the complete authoritative scan path.
3. Keep Stable-PK Secondary Index V2 as an independently gated conditional stage (`FR-031`).

---

## 8. Bounded Background Flush, Compaction, and Transfer Work (FR-022–FR-025, SC-006, SC-007)

### Decision
1. Decouple `ManifestService::mark_pending_write` from `flush_scope_locks` and hold the scope publication lock in `FlushScopeWriter::write_scope` only during microsecond batch reservation and final manifest CAS swap (moving Parquet encoding/upload/rename and `scope_hook.after_scope_write` outside the lock).
2. Reclaim idle `flush_scope_locks` entries when no holder/waiter references remain (`FR-025`).
3. Stream Parquet segments directly to/from disk-backed ZIP archives inside `tokio::task::spawn_blocking` with fixed buffers in `kalamdb-transfer` and `kalamdb-jobs` (`FR-023`, `FR-024`).

---

## 9. Shared SQL Semantics, Contextual Plan Cache, and Singleflight Planning (FR-026–FR-029, SC-008)

### Decision
1. Include `postgres_star: bool`, `search_path`, `session_settings`, `parameter_oids`, `principal`, and epochs in `PlanCacheKey` (`FR-027`).
2. Upgrade `PlanCache` inflight tracking to singleflight coordination (`LeaderGuard` + `tokio::sync::Notify`) with recursive plan byte estimation and table-scoped invalidation (`FR-026`, `FR-028`).
3. Replace `returning_describe_sql` string formatting/reparsing with AST construction (`returning_describe_statement`) and delete manual `extract_select_from_table_id` string splitting (`FR-029`).

---

## 10. Row V2 Compatibility Gate & Typed Job Orchestration (FR-030–FR-034, SC-011)

### Decision
1. Gate `ROW_V2_WRITERS_ENABLED` behind a durable cluster compatibility floor (`FormatCompatibilityFloor`) verified across all eligible replicas (`FR-030`).
2. Replace `job_manager: OnceCell<Arc<dyn Any + Send + Sync>>` in `AppContext` with a typed job manager trait capability and exhaustive startup executor registration check (`FR-034`).
