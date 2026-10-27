# Contract: Unified DataFusion TableProvider, Versioned Storage Engine, and Crate Boundaries

**Feature**: `039-unified-table-transactions`  
**Covers**: FR-001–FR-004, FR-019–FR-021, FR-033, FR-038–FR-047, SC-005, SC-013, SC-014, SC-015

---

## 1. Crate & Module Ownership Contract

| Crate & Module | Owns | Crate Boundary & Dependency Rules |
| :--- | :--- | :--- |
| **`kalamdb-datafusion-sources`** | Shared DataFusion `SourceProvider`, `ScanDescriptor`, `DeferredBatchExec`, `DeferredBatchSource`, MVCC winner selection (`select_latest_versions`, `ParquetBatchDecoder`), and DataFusion DML execution-plan primitives (`rows_affected_plan`, `dml_scan_projection`, `projected_schema`, `PreparedUpdateAssignments`, `collect_input_rows`). | Depends only on `arrow`, `datafusion`, and `kalamdb-commons`. Used by `kalamdb-system`, `kalamdb-tables`, `kalamdb-views`, `kalamdb-vector`, `kalamdb-transactions`. |
| **`kalamdb-macros`** | `#[table]` schema generator. Emits `definition() -> TableDefinition`, memoized `arrow_schema()`, primary-key column, and ordinal column order for persisted system models, virtual `system.*` rows, `pg_catalog` rows, and Kalam `information_schema` rows. | Schema authoring stays here. Provider crates consume the generated definition. Do not add a second schema language. |
| **`kalamdb-system`** | Persisted `system.*` stores and `KeyedTableProvider<K, V>`. SQL `SELECT` only. Runtime `get` / `scan_index` / `put` stay on `IndexedEntityStore` and existing caches. `build_record_batch!` stays. | Must NOT depend on `kalamdb-tables`. Retires `SystemTableScan`, `SimpleSystemTableScan`, `impl_indexed_system_table_provider!`, `impl_simple_system_table_provider!`, `impl_system_table_provider_metadata!`, the 11 `catalog/*_provider.rs` files, and `SecuredSystemTableProvider` wrapping. Does not retire `build_record_batch!`. |
| **`kalamdb-views`** | `ComputedCatalogProvider` for virtual `system.*`, `pg_catalog.*`, and Kalam `information_schema` extensions. Stock DataFusion `information_schema` stays for relations Kalam does not own. | Must NOT depend on `kalamdb-tables`. Projections stay in memory. Hand-written `Field::new` schemas and hand-built `ColumnDefinition` lists are replaced by `#[table]`. |
| **`kalamdb-tables`** | Unified `KalamTableProvider` (implementing DataFusion `TableProvider` once for `User`, `Shared`, and `Stream`), `TableProviderCore`, `TableCapabilityProfile`, `VersionedTableEngine` (serving both `User` and `Shared` tables over `ScopedRowKey`/`VersionedTableRow`), `StreamTableEngine` (serving `Stream` tables over `StreamTableStore`), and `SharedTableAuthorization`. | Retires `BaseTableProvider<K, V>`, `DeferredMvccScanProvider<K, V>`, `UserTableProvider`, `SharedTableProvider`, `StreamTableProvider`, `UserTablePkIndex`, `SharedTablePkIndex`, `UserRowCodec`, and `SharedRowCodec`. Moves `topics/` out to `kalamdb-publisher`. |
| **`kalamdb-store`** | `StorageBackend`, `EntityStore`, `IndexedEntityStore`, `PrefixIndex` (with `IndexRole::PrimaryKey` vs `IndexRole::SecondaryScalar`), snapshot-aware `push_replace`, and operation-buffer builders (`append_insert_ops`, `append_replace_ops`, `append_delete_ops`). | Owns all RocksDB and key-value index mechanics. Never commits per-table batches independently during multi-table transaction preparation. |
| **`kalamdb-flush`** | `ManifestService`, `FlushScopeWriter`, `VersionedTableFlushJob` (unified hot-to-cold flush for `User` and `Shared` scopes), and `small_segment` compaction. | Replaces separate `UserTableFlushJob` (`flush/users.rs`) and `SharedTableFlushJob` (`flush/shared.rs`) with one scope-aware `VersionedTableFlushJob`. |
| **`kalamdb-publisher`** | `TopicPublisherService`, `TopicMessageStore`, `TopicMessage`, `topic_message_schema`, and topic retention index. | Absorbs `topics/` from `kalamdb-tables` and removes the `kalamdb-tables` dependency from `kalamdb-publisher/Cargo.toml`. |
| **`kalamdb-transfer`** | `TransferError`, streaming TAR/ZIP backup, restore, table export/import, and user data export archive I/O. | Removes the `kalamdb-core` dependency from `kalamdb-transfer/Cargo.toml`. |

---

## 2. Unified DataFusion `KalamTableProvider` Contract

`KalamTableProvider` is the **single** DataFusion `TableProvider` implementation registered in `SchemaRegistry` and DataFusion schemas for all non-system tables (`User`, `Shared`, `Stream`).

```rust
pub struct KalamTableProvider {
    core: Arc<TableProviderCore>,
    engine: TableStorageEngine,
}

pub enum TableStorageEngine {
    Versioned(VersionedTableEngine),
    Stream(StreamTableEngine),
}
```

### 2.1 Zero-Downcast Caller Access
- `CachedTableData` in `kalamdb-core::schema_registry` stores `Arc<KalamTableProvider>` directly alongside `Arc<dyn TableProvider + Send + Sync>`.
- Callers (`DmlExecutor`, `SqlExecutor`, `FlushExecutor`, `TableExportExecutor`, `UserExportExecutor`, `StreamEvictionExecutor`, `OperationService`, `LiveAuthorizationBinderAdapter`, `SharedTableRepository`) call typed methods on `KalamTableProvider` / `VersionedTableEngine` / `StreamTableEngine` directly — **zero** `.as_any().downcast_ref::<UserTableProvider | SharedTableProvider | StreamTableProvider>()` calls remain in production code.

### 2.2 Single `TableProvider` Implementation Rules
1. **`TableProvider::scan(state, projection, filters, limit)`**:
   - Enforces table access (`check_user_table_access` for `User`/`Stream`; `check_shared_table_access` for `Shared`), `validate_transaction_table_access`, and `ensure_leader_read` once.
   - Builds `ScanDescriptor`, physical residual filter, and `output_projection` remapping once.
   - Dispatches execution to `DeferredBatchExec` (wrapping `VersionedTableEngine` or `StreamTableEngine`), and wraps in `TransactionOverlayExec` with minimal column projection when an active transaction overlay exists for `self.core.table_id()`.
2. **`TableProvider::insert_into(state, input, insert_op)`**:
   - Enforces write access, `InsertOp::Append`, and leader write routing once.
   - Collects input rows, validates foreign keys (`reject_missing_foreign_keys`), and checks RLS `WITH CHECK` policies when `rls_mode == ForcePolicyCheck`.
   - Stages into active transaction (for `Versioned`) or executes autocommit batch insert (`Versioned` / `Stream`), returning `rows_affected_plan`.
3. **`TableProvider::delete_from(state, filters)`**:
   - Enforces write access and leader write routing once.
   - Resolves matching PKs via `pk_membership_literals` fast path (when RLS permits) or `collect_matching_rows_with_projection`.
   - Stages into active transaction (with `pk_claims` + `committed_primary_key_is_live`) or executes autocommit batch delete (`Versioned` soft-delete tombstones vs `Stream` hard-delete).
4. **`TableProvider::update(state, assignments, filters)`**:
   - Rejects `Stream` tables immediately via `capabilities.supports_update()`.
   - Computes minimal DML scan projection (`dml_scan_projection`), collects matching rows, evaluates assignments in batches via `PreparedUpdateAssignments`, enforces RLS `WITH CHECK`, handles PK relocation (`Delete(old_pk)` + `Insert(new_pk)`) and no-op filtering, and stages or executes batch updates.

### 2.3 Persisted `System` Tables (`KeyedTableProvider<K, V>` in `kalamdb-system`)
Persisted `system.*` tables stay **shared-scope, direct-keyed (`K -> V`), hot-only, SQL-read-only**. They do not use `VersionedTableEngine`.

1. **Runtime path (must not get slower)**:
   - Authentication, jobs, schema registry, manifest, and catalog services call `IndexedEntityStore` (`get`, `scan_index`, `put`, `update_with_old`, `delete`) and the caches already in front of those stores.
   - That path does not build a logical plan, a `RecordBatch`, a `Row`, or an overlay.
2. **SQL `SELECT` path**:
   - `KeyedTableProvider<K, V>` implements `TableProvider` once. `check_system_table_access` runs inside `scan`.
   - Primary-key equality calls `store.get` and encodes one row with `build_record_batch!`.
   - Secondary predicates use `find_best_index_for_filters` and the existing index iterator.
   - A full scan encodes with `build_record_batch!` in one pass from the typed value and the `#[table]` column order.
   - Projection and `LIMIT` apply to that batch. There is no MVCC winner selection and no transaction overlay.
3. **SQL DML**:
   - `insert_into`, `delete_from`, and `update` return `"Cannot modify system table via SQL DML"`. Explicit SQL transactions reject `TableType::System`.
4. **What is deleted**:
   - The 11 `providers/catalog/*_provider.rs` files, `catalog/scan.rs`, `SystemTableScan`, `SimpleSystemTableScan`, and the three `impl_*_system_table_provider!` macros.
   - `build_record_batch!` is not deleted.

### 2.4 Computed Catalog Tables (`ComputedCatalogProvider` in `kalamdb-views`)
`pg_catalog`, virtual `system.*` views, and Kalam-owned `information_schema` relations are system catalog tables.

1. One provider shell owns access checks, projection, limit, and the schema from `#[table]`.
2. Each relation keeps its current in-memory projection (`PgCatalogView::compute_batch`, `VirtualView` producers, session/stats callbacks).
3. These relations are not inserted into `IndexedEntityStore`, not versioned, and not flushed.
4. Hand-written `schema()` field lists (for example `pg_class`) and hand-built `ColumnDefinition` lists (for example `system.sessions`) are removed after `#[table]` emits the same definition and Arrow schema.
5. DataFusion's stock `information_schema` provider remains for relations Kalam does not own.
6. A representative introspection query (namespace, table, and column lookup used by wire clients) must meet SC-015.

### 2.5 Schema Macro Contract
`#[table]` is the only schema authoring path for the relations in §2.3 and §2.4.

- It emits `definition() -> TableDefinition`.
- It emits `arrow_schema() -> SchemaRef` as a `OnceLock` of `definition().to_arrow_schema()`.
- It emits the primary-key column name and the ordinal column order.
- `build_record_batch!` consumes that order. A relation must not keep a second field list that can drift from the macro.

---

## 3. Unified `VersionedTableEngine` & Shared Fast Paths Contract (FR-002, FR-040, FR-041)

Because `VersionedTableEngine` operates over `ScopedRowKey` (`User(UserId, VersionId)` | `Shared(VersionId)`) and `VersionedTableRow`:
1. **Shared Hot-Storage Fast Paths for Both `User` and `Shared` Tables**:
   - `columnar_hot_scan(scope, snapshot, projection, filter, limit, keep_deleted)`: decodes projected KOBJ columns straight into Arrow for both user-scoped prefix scans and shared table scans when no cold Parquet segments exist.
   - `sum_hot_covering_integers(scope, filters, value_column)`: computes `COUNT`/`SUM`/`AVG` directly from secondary prefix index keys for both user-scoped and shared tables.
   - `plan_literal_equality_updates(scope, ...)`: plans single-equality indexed updates without full DataFusion planning for both user-scoped and shared tables.
2. **Pure Preparation vs Execution (`TableMutationAccumulator`)**:
   - `prepare_insert_batch`, `prepare_update_batch`, `prepare_delete_batch`, and `prepare_encoded_field_inserts` validate constraints and append `kalamdb_store::Operation`s + deferred side effects into `TableMutationAccumulator` without executing per-row commits or premature side effects.

---

## 4. Index Role & Tombstone Contract (FR-019–FR-021)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexRole {
    /// Index 0 on versioned tables: indexes both live rows and tombstones so
    /// point lookups and uniqueness checks observe hot tombstones in O(1).
    PrimaryKey,
    /// User-defined / secondary scalar indexes: indexes live rows only (`_deleted == false`).
    SecondaryScalar,
}
```

1. `PrefixIndex` carries `role: IndexRole` (`IndexRole::PrimaryKey` for index 0, `IndexRole::SecondaryScalar` for secondary scalar indexes).
2. When `IndexedEntityStore` prepares operations for an entity with `_deleted == true`:
   - `IndexRole::PrimaryKey`: writes the new tombstone index key (and removes the prior hot version's key unless pinned by an active snapshot).
   - `IndexRole::SecondaryScalar`: removes the prior hot version's secondary index key (unless pinned by an active snapshot) and **skips** writing a new secondary index key for the tombstone.
3. Queries with `keep_deleted == true` bypass `IndexRole::SecondaryScalar` and use the authoritative main-table + cold scan path.
