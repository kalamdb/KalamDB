# Hot/Cold Storage and Query Unification

## Overview

KalamDB stores user and shared table data in two tiers:

| Tier | Engine | Contents | Scope |
|------|--------|----------|-------|
| **Hot** | RocksDB (`EntityStore` / `IndexedEntityStore`) | Recent writes, all MVCC versions | Per-table RocksDB partition |
| **Cold** | Parquet on filesystem or object storage | Flushed, deduplicated snapshots | Per table (shared) or per user (user tables) |

Every row carries MVCC metadata: `_seq`, `_commit_seq`, and `_deleted`. Both tiers can hold multiple versions of the same primary key. Reads must pick the winning version across hot and cold before returning results.

Stream tables are hot-only: no Parquet, no merge step. See [stream-storage.md](stream-storage.md).

Manifest indexing, flush mechanics, and compaction are documented in [manifest.md](manifest.md). This document focuses on the **read path** — how DataFusion table providers scan both tiers and unify results.

## Hot storage layout

### User tables

Keys are `{user_id}:{_seq}` in a RocksDB partition. A secondary PK index enables O(1) lookups by primary-key value.

```
backend/crates/kalamdb-tables/src/user_tables/user_table_store.rs
```

- `UserTableRowId`: composite of `user_id` and `_seq`
- `UserTableRow`: `user_id`, `_seq`, `_deleted`, and JSON `fields`
- Storage key format: `{user_id}:{_seq}` (big-endian bytes)
- `IndexedEntityStore` maintains the PK secondary index

### Shared tables

Same MVCC model, but without a user prefix. One RocksDB partition covers the entire table.

### Write-path side effect

On every INSERT, UPDATE, or DELETE, providers call `manifest_service.mark_pending_write(table_id, user_id?)` to signal that hot storage has unflushed data. Normal DML does not rewrite `manifest.json`; see [manifest.md](manifest.md) for sync states.

## Cold storage and manifest

Cold data lives in Parquet segment files. A `manifest.json` per scope `(table_id, user_id?)` tracks segments with path, `min_seq`/`max_seq`, `column_stats`, schema version, and tombstone status.

`ManifestService` is the canonical read path (memory → RocksDB → cold storage). On query, `utils/parquet.rs` calls `get_or_load_async` before opening any Parquet file.

If the manifest has zero segments, the cold path is skipped entirely — no storage listing, no file I/O.

## DataFusion query path

### 1. `TableProvider::scan()` builds a deferred plan

`UserTableProvider::scan` and `SharedTableProvider::scan` call `base_scan_with_overlay`, which produces a `DeferredBatchExec`. Scan work is deferred until execution time, not planning time.

```
UserTableProvider::scan()
  → base_scan_with_overlay()
    → base_scan()
      → DeferredMvccScanSource wrapped in DeferredBatchExec
```

Location: `backend/crates/kalamdb-tables/src/utils/base.rs`

At plan time, `base_scan()`:

- Validates leader and transaction access
- Classifies filters (PK equality, `_seq` bounds, etc.)
- Builds filter pushdown and projection remapping
- Captures scan context (user scope, transaction snapshot commit seq)

### 2. Execute time: `scan_rows_output()`

When DataFusion executes the plan, `DeferredMvccScanSource` calls `scan_rows_output()`:

1. **PK fast path** — filter is `pk = literal` and no transaction snapshot: hot PK index + targeted cold lookup; skip full scan
2. **COUNT fast path** — empty projection, no filter: metadata-only hot+cold merge
3. **Full scan** — `scan_kvs_with_context()` → merge → `rows_to_arrow_batch()`

### 3. Concurrent hot + cold fetch

Providers launch hot and cold scans in parallel via `tokio::join!` inside `resolve_latest_scan_from_futures`.

**User table** (single-user scan):

- **Hot**: `store.scan_with_raw_prefix_async(&user_prefix, …)` — all MVCC versions for the session user in RocksDB
- **Cold**: `scan_parquet_files_as_result_async(user_id, filter, cold_columns)` — manifest-driven Parquet read

**Shared table**:

- **Hot**: `store.scan_typed_with_prefix_and_start_async(None, …)` — full partition scan
- **Cold**: same Parquet path with `user_id: None`

Location: `user_table_provider.rs`, `shared_table_provider.rs`, `utils/base.rs`

### 4. Cold path: manifest-driven Parquet scan

`utils/parquet.rs` → `ManifestAccessPlanner::scan_parquet_files_async`:

1. Load manifest via `ManifestService::get_or_load_async`
2. Fast exit if manifest has no segments
3. Select segment files:
   - all files (`plan_all_files`), or
   - pruned by `_seq` range (`plan_by_seq_range`), or
   - pruned by PK min/max (`plan_by_pk_value`)
4. Open Parquet streams concurrently
5. Apply column projection and schema evolution
6. Concatenate into one `RecordBatch`

If the manifest is missing or corrupt, **degraded mode** lists the directory and scans all `*.parquet` files.

Location: `backend/crates/kalamdb-tables/src/manifest/planner.rs`

## Unification: MVCC winner selection

The core merge lives in `kalamdb-datafusion-sources`. `resolve_latest_scan_from_futures` (in `utils/base.rs`) joins hot and cold results, then calls `resolve_latest_kvs_from_cold_batch`.

### Algorithm

1. **Hot rows** become `VersionCandidate { pk_key, commit_seq, seq_id, deleted, payload }`.
2. **Cold rows** decode metadata first (`_seq`, `_commit_seq`, `_deleted`, PK) — full row materialization is delayed.
3. `select_latest_versions()` groups by PK and picks the winner by `(commit_seq, seq_id)` ordering (higher wins).
4. Tombstones (`_deleted = true`) are filtered out unless the query explicitly requests them.
5. Transaction snapshot reads filter rows where `commit_seq <= snapshot_commit_seq`.
6. Only **winning cold rows** are fully decoded via the provider's `build_cold_row` callback.

Location: `backend/crates/kalamdb-datafusion-sources/src/exec.rs`

### Why reads are not streamable

User and shared tables require seeing all versions before returning any row: multiple versions per PK exist, and the reader must find `MAX(_seq)` (tie-broken by `commit_seq`) per primary key while respecting tombstones. True iterator-based streaming is therefore not possible for these table types.

Stream tables bypass this path entirely — append-only, hot-only, TTL-based eviction.

```text
Hot Storage (RocksDB) ─┐
                       ├──> Merge ──> Version Resolution ──> Filter Deleted ──> Result
Cold Storage (Parquet) ┘       (requires ALL rows to find MAX per PK)
```

### PK fast path

When the filter is `pk = literal` and no transaction snapshot is active:

1. Check hot PK index — if the latest hot version is a tombstone, return nothing (cold is suppressed)
2. Otherwise compare latest hot vs latest cold for that PK
3. Skip the full table scan

Tombstone masking prevents cold Parquet from surfacing a row already deleted in hot storage.

### Admin "all users" scan

When a user-table reader has cross-user access, `scan_all_users_with_version_resolution_async`:

1. Scans all hot rows
2. Discovers user scopes from manifest metadata and hot data
3. Merges hot + cold **per user**
4. Concatenates results

## Hot → cold: flush (summary)

Periodic flush jobs move data from RocksDB to Parquet. Full detail is in [manifest.md](manifest.md).

1. Scan hot storage in key order (user tables group by `user_id`)
2. Deduplicate per PK — keep only the latest version (same winner logic as reads)
3. Write Parquet via `FlushScopeWriter` (atomic: `.tmp` → rename)
4. Delete flushed keys from RocksDB
5. `persist_flushed_segment()` — append `SegmentMetadata`, write `manifest.json`, refresh RocksDB and memory cache

After flush, new writes land in hot again (`mark_pending_write`). Reads always merge whatever is currently in both tiers.

Latest tombstones stay hot after flush so they continue masking older cold segments.

## End-to-end read flow

```text
SQL SELECT
    │
    ▼
UserTableProvider::scan() / SharedTableProvider::scan()
    │
    ▼
base_scan() → DeferredBatchExec (deferred until execute)
    │
    ▼
scan_rows_output()
    ├─ PK fast path? → hot PK index + cold PK prune
    ├─ COUNT(*)?     → metadata-only hot+cold merge
    └─ full scan:
           ├─ HOT: RocksDB prefix scan (all MVCC versions)
           │        tokio::join!
           └─ COLD: manifest → prune segments → Parquet streams
                    │
                    ▼
           resolve_latest_kvs_from_cold_batch()
           (group by PK, max(commit_seq, seq), drop tombstones)
                    │
                    ▼
           RecordBatch → DataFusion continues (filter, project, limit)
```

## Differences by table type

| | User | Shared | Stream |
|---|------|--------|--------|
| Hot key | `{user_id}:{seq}` | `{seq}` | `{user_id}:{seq}` |
| Cold path | `user/{ns}/{table}/{userId}/` | `shared/{ns}/{table}/` | none |
| Manifest scope | per `(table, user)` | per `(table, None)` | n/a |
| MVCC merge | yes | yes | no (append-only, hot only) |
| RLS | session `user_id` scopes hot+cold | none | session `user_id` |

## Key source files

Read in this order to trace the implementation:

| File | Role |
|------|------|
| `backend/crates/kalamdb-tables/src/utils/base.rs` | `BaseTableProvider`, `DeferredMvccScanProvider`, `base_scan`, `resolve_latest_scan_from_futures` |
| `backend/crates/kalamdb-datafusion-sources/src/exec.rs` | `select_latest_versions`, `resolve_latest_kvs_from_cold_batch`, `DeferredBatchExec` |
| `backend/crates/kalamdb-tables/src/utils/parquet.rs` | Cold scan orchestration, manifest integration |
| `backend/crates/kalamdb-tables/src/manifest/planner.rs` | Segment pruning, Parquet streaming |
| `backend/crates/kalamdb-tables/src/user_tables/user_table_provider.rs` | User-specific hot scan, `TableProvider::scan` |
| `backend/crates/kalamdb-tables/src/shared_tables/shared_table_provider.rs` | Shared-specific hot scan |
| `backend/crates/kalamdb-flush/src/service.rs` | `ManifestService`, `persist_flushed_segment` |
| `backend/crates/kalamdb-flush/src/flush/users.rs` | User table flush (hot → Parquet) |

## Related documentation

- [manifest.md](manifest.md) — manifest tiers, flush, compaction, recovery
- [stream-storage.md](stream-storage.md) — hot-only stream tables
- [development/user-table-storage.md](../development/user-table-storage.md) — user table storage layout

## Core invariant

Both tiers store versioned rows. Reads always pick the latest visible `(commit_seq, seq)` per primary key across hot RocksDB and cold Parquet, with tombstones winning over older live rows.
