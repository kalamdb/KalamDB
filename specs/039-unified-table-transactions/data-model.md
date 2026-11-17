# Phase 1 Data Model: Unified Table Processing and Atomic Transaction Optimization

**Feature**: `039-unified-table-transactions`  
**Date**: 2026-10-27  
**Spec**: [spec.md](spec.md) | **Research**: [research.md](research.md)

---

## 1. Unified Provider & Storage Entities

### 1.1 `KalamTableProvider`, `TableStorageEngine`, and `KeyedTableProvider<K, V>` (FR-001–FR-004, FR-038–FR-046)

Unified DataFusion `TableProvider` architecture across all four table kinds (`User`, `Shared`, `Stream`, and persisted `System` tables):

```text
KalamTableProvider (implements datafusion::datasource::TableProvider + SourceProvider in kalamdb-tables)
├── core: Arc<TableProviderCore>
│   ├── table_def: Arc<TableDefinition>
│   ├── table_id: TableId
│   ├── capabilities: TableCapabilityProfile
│   ├── schema: SchemaRef                     # Memoized Arrow schema
│   ├── storage_schema: Arc<StorageSchema>    # Historical + current ordinal KOBJ layout
│   ├── primary_key_field_name: String
│   ├── primary_key_column_id: u64
│   ├── is_auto_increment_pk: bool
│   ├── column_defaults: HashMap<String, Expr>
│   ├── non_null_columns: HashSet<String>
│   └── services: Arc<TableServices>
└── engine: TableStorageEngine
    ├── Versioned(VersionedTableEngine)       # Serves BOTH TableType::User and TableType::Shared
    │   ├── store: Arc<VersionedIndexedStore> # IndexedEntityStore<ScopedRowKey, VersionedTableRow>
    │   ├── vector_columns: Vec<(String, u32)>
    │   ├── vector_stores: HashMap<String, Arc<VersionedVectorHotStore>>
    │   └── authorization: Option<SharedTableAuthorization> # Present when rls_mode == ForcePolicyCheck
    └── Stream(StreamTableEngine)             # Serves TableType::Stream
        ├── store: Arc<StreamTableStore>
        └── ttl_seconds: Option<u64>

KeyedTableProvider<K, V> (persisted system.* SQL SELECT only; kalamdb-system)
├── table_id: TableId                         # e.g., system.users, system.jobs, system.types
├── table_def: Arc<TableDefinition>           # From #[table] definition(), not a second schema
├── schema: SchemaRef                         # OnceLock of definition().to_arrow_schema()
├── store: IndexedEntityStore<K, V>           # Direct-keyed RocksDB store + secondary indexes
└── encode: build_record_batch! binding       # One-pass typed value -> Arrow. No Row round-trip

Runtime callers do not go through KeyedTableProvider. Auth, jobs, schema registry, and
catalog services call IndexedEntityStore::get / scan_index / put and their existing caches.

ComputedCatalogProvider (virtual system.*, pg_catalog.*, Kalam information_schema)
├── table_def: Arc<TableDefinition>           # From #[table(namespace = ...)]
├── schema: SchemaRef                         # Same OnceLock as persisted tables
└── project: specialized in-memory compute    # Not RocksDB, not MVCC, not flushed
```

---

### 1.2 `TableScope`, `ActorContext`, and `ScopedRowKey` (FR-002, FR-003, FR-040, FR-041)

Separates data ownership from the authenticated principal while unifying the hot-storage key and row model for `User` and `Shared` tables.

```text
TableScope
├── User(UserId)     # Required for TableType::User and TableType::Stream
└── Shared           # Required for TableType::Shared

ActorContext
├── actor_user_id: Option<UserId>  # Authenticated subject (for RLS, CDC/topic actor attribution)
├── role: Role                     # Subject role (User, Service, Dba, System)
└── read_context: ReadContext      # Client vs Internal routing requirement

ScopedRowKey (implements StorageKey + PrefixIndexedKey)
├── User(UserTableRowId)           # Encodes (user_id, version) via storekey — bit-identical to existing user_ tables
└── Shared(SharedTableRowId)       # Encodes (version) via storekey — bit-identical to existing shared_ tables

VersionedTableRow (implements KSerializable + ScanRow + VersionedRow + PrefixIndexedValue)
├── scope: TableScope
├── _version: VersionId
├── _deleted: bool
└── fields: Row
```

**Validation & Compatibility Rules**:
- `TableType::User | TableType::Stream` + `TableScope::Shared` (or missing `UserId`) -> **Rejected (`InvalidOperation`)** before any storage read or write.
- `TableType::Shared` + `TableScope::User(_)` -> **Rejected (`InvalidOperation`)** for storage key construction; `ActorContext.actor_user_id` is passed separately to `SharedTableAuthorization` and `ChangeNotification::with_actor`.
- On-disk key and value encodings for `ScopedRowKey` and `VersionedTableRow` are 100% byte-compatible with existing `UserTableRowId`/`UserTableRow` and `SharedTableRowId`/`SharedTableRow` partitions (`FR-032`).

---

### 1.3 `TableCapabilityProfile` (FR-004, FR-038, FR-043)

Declarative capability profile across all four table kinds (`User`, `Shared`, `Stream`, `System`).

| Field | `User` Table | `Shared` Table | `Stream` Table | Persisted `system.*` | Computed catalog (`pg_catalog`, virtual `system.*`, Kalam `information_schema`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `scope_mode` | `ScopeMode::UserRequired` | `ScopeMode::Shared` | `ScopeMode::UserRequired` | `ScopeMode::Shared` (Global) | `ScopeMode::Shared` (Global) |
| `mutation_mode` | `MutationMode::VersionedMvcc` | `MutationMode::VersionedMvcc` | `MutationMode::AppendAndHardDelete` | `MutationMode::DirectApiOnly` (SQL DML rejected; typed API hits `IndexedEntityStore`) | `MutationMode::ReadOnly` (no store) |
| `cold_tier_mode` | `ColdTierMode::ParquetManifest` | `ColdTierMode::ParquetManifest` | `ColdTierMode::None` | `ColdTierMode::None` | `ColdTierMode::None` |
| `transaction_mode` | `TransactionMode::ExplicitAndAutocommit` | `TransactionMode::ExplicitAndAutocommit` | `TransactionMode::AutocommitOnly` | `TransactionMode::None` | `TransactionMode::None` |
| `version_source` | `VersionSource::RaftEntryOrdinal` | `VersionSource::RaftEntryOrdinal` | `VersionSource::LocalMonotonicWithTimestamp` | `VersionSource::DirectKeyInPlace` | `VersionSource::None` (recomputed each scan) |
| `rls_mode` | `RlsMode::OwnerScopePrefix` | `RlsMode::ForcePolicyCheck` | `RlsMode::OwnerScopePrefix` | `RlsMode::SystemAdminOnly` | `RlsMode::SystemAdminOnly` (role already applied inside today's `pg_catalog` projections stays there) |
| `scan_engine` | `VersionedTableEngine` | `VersionedTableEngine` | `StreamTableEngine` | `KeyedTableProvider`: PK equality is `store.get`; scans use `build_record_batch!` | `ComputedCatalogProvider`: in-memory projection only |
| `schema_source` | Table definition registry | Table definition registry | Table definition registry | `#[table]` on the model | `#[table]` on the projection row |

---

### 1.4 `ScopedTargetKey` and `AuthoritativeMutationTarget` (FR-011, FR-013, FR-015)

Canonical identity and resolved prior state for a single row target within a transaction or batch DML operation.

```text
ScopedTargetKey
├── table_id: TableId
├── scope: TableScope              # User(UserId) or Shared
└── pk_bucket: PkBucketKey         # Canonical typed PK representation (Int64, Utf8, FixedSizeBinary(16), etc.)

AuthoritativeMutationTarget
├── key: ScopedTargetKey
├── schema_version: u32
├── baseline_state: TargetBaselineState
│   ├── HotLive { storage_key: ScopedRowKey, row: VersionedTableRow }
│   ├── HotTombstone { storage_key: ScopedRowKey, row: VersionedTableRow }
│   ├── ColdLive { row: VersionedTableRow }
│   └── Absent
└── expected_max_version: Option<VersionId>  # Snapshot frontier or scanned version for conflict detection
```

**Validation Rules**:
- If `expected_max_version` is `Some(bound)` and `baseline_state` has `version > bound`, commit preparation fails with a write/serialization conflict (`FR-011`).
- `TargetResolutionCache` stores `HashMap<ScopedTargetKey, AuthoritativeMutationTarget>` per preparation pass so each distinct `(table_id, scope, pk_bucket)` is loaded at most once from hot/cold storage (`FR-015`).

---

### 1.5 `CommitFoldEntry` and State Transition Matrix (FR-013, FR-014, FR-018)

Represents the folded effect of multiple mutations to the same `ScopedTargetKey` within a transaction commit domain.

```text
FoldedTargetState
├── Insert { fields: Row, from_tombstone: bool }
├── Update { patch_fields: BTreeMap<String, ScalarValue>, full_merged_fields: Option<Row> }
├── Delete { prior_known_fields: Option<Row>, was_inserted_in_tx: bool }
└── NoOpAfterInsertDelete { validate_absence: bool }
```

#### State Transition Rules

| Current `FoldedTargetState` | Next `StagedMutation` | Resulting `FoldedTargetState` | Statement & History Contract |
| :--- | :--- | :--- | :--- |
| *(None / Missing)* | `Insert(row)` | `Insert { fields: row, from_tombstone: false }` | Validates PK absence/uniqueness at commit. |
| `Insert { fields, from_tombstone }` | `Update(patch)` | `Insert { fields: merge(fields, patch), from_tombstone }` | Preserves `UPDATE` statement affected count; merges explicit `Null` and non-null columns in order. |
| `Update { patch_fields, .. }` | `Update(patch)` | `Update { patch_fields: merge(patch_fields, patch), .. }` | Preserves untouched columns; explicit `Null` overwrites earlier patch value. |
| `Insert { from_tombstone: false, .. }` | `Delete` | `NoOpAfterInsertDelete { validate_absence: true }` | No final live row or tombstone if key was absent before tx; validates key was not concurrently or previously live; emits complete-event history if required by active cursor contract. |
| `Insert { from_tombstone: true, .. }` | `Delete` | `Delete { prior_known_fields, was_inserted_in_tx: true }` | Key had a pre-existing tombstone/row before re-insert; final state remains a tombstone masking any cold history. |
| `Update { patch_fields, .. }` | `Delete` | `Delete { prior_known_fields: merged_before_delete, was_inserted_in_tx: false }` | Tombstone stores full last-before-delete payload (authoritative base row + `patch_fields`). |
| `Delete { .. }` | `Insert(row)` | `Insert { fields: row, from_tombstone: true }` | Validated reinsertion; replaces hot tombstone in same atomic batch. |
| `NoOpAfterInsertDelete { .. }` | `Insert(row)` | `Insert { fields: row, from_tombstone: false }` | Validated reinsertion after in-tx insert+delete. |
| `Insert { .. }` | `Insert(_)` | **Error (`AlreadyExists`)** | Duplicate primary key violation unless statement had `ON CONFLICT` resolved prior to staging. |
| `Delete { .. }` or `NoOpAfterInsertDelete` | `Update(_)` | Unchanged (`Delete` / `NoOpAfterInsertDelete`) | Zero rows affected (or `NotFound` for direct PK update); never invents a partial row. |
| `Delete { .. }` | `Delete` | Unchanged (`Delete`) | Idempotent final tombstone. |

**Primary-Key Rename Rule**:
- An `UPDATE` that changes the primary key column from `pk_old` to `pk_new` (`pk_old != pk_new`) is expanded in statement order into `Delete(pk_old)` + `Insert(pk_new, full_merged_row)` before entering the transition table above.

---

### 1.6 `PreparedCommitBatch` and `PostCommitEffectPlan` (FR-006–FR-008, FR-012)

The output of pure transaction/batch preparation and the input to the single atomic storage commit.

```text
PreparedCommitBatch
├── commit_domain: GroupId
├── log_index: u64
├── storage_operations: Vec<kalamdb_store::Operation>
│   ├── Main table row Put / Delete ops (all touched tables in domain)
│   ├── Primary-key index Put / Delete ops (all touched tables)
│   ├── Secondary scalar index Put / Delete ops (tombstones omitted per FR-020)
│   └── Manifest pending-write index Put ops (user/shared scopes)
├── rows_affected: usize
└── post_commit_effects: PostCommitEffectPlan

PostCommitEffectPlan
├── group_frontier_advance: Option<(GroupId, u64)>
├── manifest_memory_updates: Vec<(TableId, Option<UserId>)>
├── vector_staging_batches: Vec<PreparedVectorBatch>
├── notifications: Vec<ScopedNotificationDispatch>
└── file_cleanup_intents: Vec<DeferredFileCleanup>
```

**Invariants**:
- If `storage_operations.is_empty()`, **zero** `StorageBackend::batch` calls are made.
- If `storage_operations` is non-empty, **exactly one** `StorageBackend::batch(storage_operations)` call is executed for the entire commit.
- `post_commit_effects` are executed **strictly after** `StorageBackend::batch` returns `Ok(())`.

---

### 1.7 `SnapshotFrontier` and Hot-Version Retention (FR-009, FR-019–FR-021)

```text
CommitSequenceTracker
├── frontiers_by_group: DashMap<GroupId, AtomicU64>
└── active_pins_by_group: DashMap<GroupId, BTreeMap<u64, usize>>  # log_index -> active pin refcount
```

---

### 1.8 `PublicationGeneration` & `FlushScopeCoordination` (FR-022, FR-023, FR-025)

```text
ScopeCoordinationEntry
├── reservation_lock: Mutex<()>         # Held only during microsecond batch reservation & final manifest CAS
├── active_flush_generation: AtomicU64  # Monotonic generation counter per scope
└── active_holders: AtomicUsize         # Refcount for safe DashMap reclamation when == 0
```

---

### 1.9 `PlanCacheKey` & `CachedPlanEntry` (FR-026–FR-029)

```text
PlanCacheKey
├── namespace: SchemaId
├── role: Role
├── sql: String
├── database: String
├── search_path: String
├── postgres_star: bool                 # Distinguishes PGWire vs HTTP SQL wildcard projection
├── parameter_oids: Vec<u32>
├── session_settings: String
├── parser_revision: u64
├── dependency_epoch: u64
├── permission_epoch: u64
└── principal: Option<UserId>

CachedPlanEntry
├── plan: Arc<LogicalPlan>
├── referenced_tables: SmallVec<[TableId; 2]>
└── estimated_bytes: u64
```

---

### 1.10 `FormatCompatibilityFloor` (FR-030–FR-032)

```text
FormatCompatibilityFloor
├── min_supported_row_format: u8        # 1 = KOBJ Row V1, 2 = KOBJ Row V2
├── row_v2_writer_enabled: bool         # Only true after all eligible cluster nodes report reader >= 2
└── secondary_index_format_version: u8  # Independently gated from row format
```

---

## 2. Raft Group Entities (User Story 8, FR-048–FR-056)

Specified by [contracts/openraft-010.md](contracts/openraft-010.md). Every persisted byte layout below is owned by KalamDB in `kalamdb-raft`. No OpenRaft type is bincode-encoded directly into RocksDB or a snapshot file.

### 2.1 `KalamTypeConfig` and `RaftCommandBytes` (FR-048)

```text
KalamTypeConfig (OpenRaft 0.10)
├── NodeId   = u64
├── Node     = KalamNode
├── Term     = u64
├── LeaderId = leader_id_std::LeaderId<u64, u64>   # committed leader id = term
├── D        = RaftCommandBytes                    # Debug + Display: command kind and byte length only
├── R        = RaftCommandBytes                    # apply response bytes
└── AsyncRuntime = tokio-rt default

RaftCommandBytes(Vec<u8>)    # same bincode layout as 0.9 Vec<u8>
```

`SnapshotData` is not on the type config in 0.10. It is the state machine's snapshot payload (section 2.3).

### 2.2 Persisted Raft records (FR-048, FR-049)

The existing types live in `backend/crates/kalamdb-store/src/raft_storage.rs`.

```text
RaftLogEntry           # existing, unchanged
├── index: u64
├── term: u64
└── payload: Vec<u8>   # tagged: Blank | Normal(RaftCommandBytes) | Membership

RaftVote               # existing, unchanged; maps to the std Vote (term, voted_for, committed)
├── term: u64
├── voted_for: Option<u64>
└── committed: bool

RaftMembership         # new, replaces the opaque bincode bytes passed to save_last_membership
├── log_id: Option<RaftLogId>       # (term, index)
├── configs: Vec<BTreeSet<u64>>     # joint configs, in order
└── nodes: BTreeMap<u64, KalamNode>
```

- `save_last_membership` / `read_last_membership` take and return `RaftMembership`, not `&[u8]`.
- Reads accept the 0.9 bincode `StoredMembership` (advanced leader id) once and drop the node id. Writes emit only `RaftMembership`. The membership payload inside a `RaftLogEntry` uses the same rule.
- A record that decodes as neither layout is a startup error, not `None`.
- Log ids are rebuilt as `(term, index)`. No node `0` placeholder.

### 2.3 Snapshot header and lifecycle (FR-056)

```text
RaftSnapshotMeta                 # existing in kalamdb-store; snapshot_id stays as the local file id
├── last_log_id: Option<RaftLogId>
├── snapshot_id: String          # local only; not sent to OpenRaft 0.10, which has no snapshot_id
└── size_bytes: u64
  + build_counter: u64           # new: next file counter, read from here instead of split('-')

StateMachineSnapshotHeader       # new, replaces the bincode StateMachineData header in raft_store.rs
├── last_applied: Option<RaftLogId>
└── last_membership: RaftMembership
```

Adding `build_counter` changes the `RaftSnapshotMeta` encoding. The reader accepts the old record (counter recovered once from the existing `snapshot_id`), and the writer emits the new one. T090 captures the old bytes.

- Retention: after a new snapshot is durable, delete files older than `max_snapshots_to_keep`.
- Install: fsync the file, then one `WriteBatch` with `RaftSnapshotMeta`, data path, `last_applied`, membership, and purge marker. Purge logs after.
- Build, current-snapshot read, and transfer share one `Arc<[u8]>` or a file stream.

### 2.4 Applied entry batch (FR-051, FR-056)

```text
AppliedEntryBatch                # one per applied Raft entry, owned by the storage
├── ops: Vec<Operation>          # table, index, and state-machine writes from apply
├── log_record: Option<RaftLogEntry>     # standalone deferred mode only
├── last_applied: (term, index)
└── after_persist: Vec<Hook>     # run once after the batch is written, before the responder
```

- Standalone mode is fixed at `RaftGroup::start` (`peers` empty). `add_learner` is rejected in that mode.
- In standalone mode a crash may lose log records above `last_applied`. That tail was never acknowledged and is not a torn tail.
- The Meta-to-Data materialization watermark stays a separate value and is never derived from `last_applied`.

### 2.5 `GroupProgressSnapshot` (FR-052)

```text
GroupProgressSnapshot            # one per local group, owned by RaftManager, refreshed from 0.10 watches
├── group_id: GroupId
├── observed_at: Instant
├── local_committed: Option<u64>
├── cluster_committed: Option<u64>
├── flushed: Option<u64>
├── applied: Option<u64>
├── snapshot: Option<u64>
├── purged: Option<u64>
├── vote: Option<(term, node_id, committed)>
├── leader: Option<u64>
└── last_quorum_acked: Option<Instant>
```

A SQL query reads the latest `GroupProgressSnapshot`. It does not start a watch. Remote applied indexes come from `peer_stats_cache` (`GetNodeInfo` RPC) and are `NULL` when absent.

### 2.6 System view columns

`system.cluster_groups` keeps its 14 existing columns in order: `group_id`, `cluster_id`, `node_id`, `group_type`, `current_term`, `vote`, `last_log_index`, `last_applied`, `snapshot`, `purged`, `committed`, `state`, `current_leader`, `millis_since_quorum_ack`. `committed` keeps its name and is filled from `GroupProgressSnapshot.cluster_committed`. Appended columns, all nullable:

| Column | Type | Source |
| --- | --- | --- |
| `local_committed` | `Int64` | `GroupProgressSnapshot.local_committed` |
| `flushed` | `Int64` | `GroupProgressSnapshot.flushed` |
| `apply_lag` | `Int64` | `cluster_committed - applied`, same group only |
| `observed_at_ms` | `Int64` | age of the snapshot in milliseconds |

`system.cluster` keeps its existing columns in order. Appended summaries: `leader_groups`, `follower_groups`, `lagging_groups`, `worst_apply_lag`, `millis_since_last_heartbeat` (existing field, now filled when known).
