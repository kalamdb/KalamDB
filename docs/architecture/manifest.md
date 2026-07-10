# Manifest and Flush Architecture

## Overview

KalamDB treats the manifest as the authoritative metadata index for cold Parquet segments. A
manifest belongs to one storage scope:

- `SHARED` tables: one manifest per table
- `USER` tables: one manifest per `(table_id, user_id)` scope

`ManifestService` is the only supported read and write path. Callers should not open
`manifest.json` directly.

Each manifest tracks:

- immutable cold segments (`SegmentMetadata`)
- per-segment `_seq` range, row count, file size, schema version, and status
- column stats keyed by stable column id for pruning and primary-key checks
- `last_sequence_number` for `batch-N.parquet` naming
- scope metadata such as FILE subfolder state and vector index pointers

## Manifest tiers

Manifest state is deliberately split across hot and cold layers.

### Shared-table scopes

```text
in-process memory cache -> RocksDB manifest copy -> storage manifest.json
```

### User-table scopes

```text
RocksDB manifest copy -> storage manifest.json
```

Shared scopes stay in process memory because they are low-cardinality and are reused by many
queries. User scopes intentionally skip memory caching so high-cardinality workloads do not create
one in-process manifest per active user.

`ManifestService::get_or_load()` is the canonical read-through API:

1. Check the fastest hot tier for the scope.
2. Fall back to the RocksDB manifest copy.
3. Fall back to storage `manifest.json`.
4. Hydrate faster layers above the source of truth that answered the request.

## Sync states and write-path behavior

The manifest cache entry has an explicit sync state:

- `in_sync`: hot tiers match storage `manifest.json`
- `pending_write`: hot data or metadata changed and storage has not been refreshed yet
- `syncing`: a flush is writing a Parquet temp file for this scope
- `stale`: hot copy should be refreshed from storage
- `error`: last sync attempt failed

Normal DML does not rewrite `manifest.json` on every row. The write path updates hot data in
RocksDB, then marks the manifest scope as `pending_write`. That keeps the hot path cheap while
still allowing flush scheduling, cold-segment pruning, and primary-key checks to see the current
scope state.

## Flush architecture

### 1. Flush discovery and scope selection

Flush jobs operate on manifest scopes with pending writes. The scheduler and jobs layer discover
those scopes through the manifest cache, then execute a user-table or shared-table flush job.

### 2. Hot-row scan and latest-version resolution

Flush reads hot rows in bounded batches using `flush.flush_batch_size`.

- `USER` tables resolve one user scope at a time because user hot keys are ordered by
  `(user_id, seq)`.
- `SHARED` tables resolve one shared scope.
- Both paths keep only the latest `_seq` for each primary-key value.
- If the primary key is missing or null, flush uses `_seq:<value>` as the fallback identity so
  version resolution still stays deterministic.

Latest tombstones are handled specially:

- tombstones are not written to Parquet during flush
- the latest tombstone key stays hot in RocksDB so it can continue masking older cold segments
- later cold-segment compaction decides whether that tombstone can be removed safely

### 3. Atomic Parquet write per scope

`FlushScopeWriter` performs the cold write under the manifest flush-scope lock:

1. Mark the manifest scope as `syncing`.
2. Read `last_sequence_number` and allocate the next `batch-N.parquet` name.
3. Write `batch-N.parquet.tmp`.
4. Atomically rename it to `batch-N.parquet`.
5. Compute segment metadata from the final batch: `_seq` range, row count, file size, schema
   version, and indexed-column stats.

The write itself is scope-local:

- shared tables write one batch file per flush scope
- user tables write one batch file per flushed user scope

### 4. Manifest commit after a successful flush

Flush does not first dirty the manifest and then separately persist it. Instead,
`FlushManifestHelper` calls `ManifestService::persist_flushed_segment()` to perform the flush-time
commit in one path:

1. Load or initialize the manifest for the scope.
2. Append the new `SegmentMetadata` entry.
3. Persist `manifest.json` to cold storage.
4. Refresh the RocksDB manifest copy.
5. Refresh the in-process cache for shared scopes.

If the cache entry still has remaining hot data for that scope, the sync state stays
`pending_write`; otherwise it becomes `in_sync`.

### 5. Hot cleanup after a successful flush

Only after the Parquet file and manifest commit succeed does flush remove hot rows. Cleanup happens
in bounded delete batches so large flushes do not build unbounded RocksDB operation vectors.

## Table Export and Import

Admin UI table transfer uses the manifest as part of the archive contract.

- `table_export` first triggers a flush for the selected `USER` or `SHARED` table scope so hot
   RocksDB rows are materialized into Parquet.
- The export ZIP contains committed Parquet segment files plus `kalamdb-table-export.json` with the
   source table definition, manifest metadata, and ZIP entry mapping.
- `table_import` accepts only this table-export ZIP format and requires the target table columns to
   match the exported table columns. It copies the Parquet files into the target table storage with
   import-specific filenames, rewrites segment IDs/paths and schema version to the target table,
   then persists the merged manifest through `ManifestService`.
- User-table transfer is scoped by `user_id`; shared-table transfer has no user scope.

The importer does not replay raw RocksDB rows. Data becomes visible through cold Parquet segments
after the target manifest is persisted.

## Post-flush small-segment compaction

KalamDB now supports optional post-flush tail compaction under `[flush.compaction]`:

```toml
[flush.compaction]
enabled = false
min_eligible_segments = 5
max_segments_per_run = 8
user_max_segment_rows = 10000
shared_max_segment_rows = 25000
```

Important boundaries:

- only `USER` and `SHARED` tables participate
- `STREAM` and `SYSTEM` tables do not use this mechanism
- the flush job itself does not rewrite old Parquet files
- post-flush compaction is leader-only job work triggered from flush scope hints

### Selection policy

`preview_small_segment_compaction()` examines the manifest tail from newest to oldest and selects
only the trailing run of segments that are all:

- readable (`status = committed`)
- smaller than the configured target for that table type
- on the same schema version

Selection stops at the first segment that is already large enough, unreadable, or on a different
schema version. This keeps compaction focused on the unstable small-file tail instead of repeatedly
rewriting older, already-good segments.

### MVCC-aware rewrite

Compaction is two-pass and scope-safe:

1. Acquire a lightweight per-scope compaction guard so duplicate `segment_compact` jobs skip.
2. Read the selected tail while the old manifest remains authoritative for queries.
3. First pass: project only primary key, `_seq`, and `_deleted` to find the latest version of each
   key inside the selected tail.
4. Check older manifest segments to determine which delete tombstones must still be preserved to
   mask older cold rows.
5. Second pass: stream only winning rows into `compact-<uuid>.parquet.tmp`.
6. Rename the temp file to `compact-<uuid>.parquet`.

### Manifest compaction semantics

The manifest itself is compacted by suffix replacement, not by rebuilding the whole scope from
scratch.

When the compacted file is ready, `ManifestService::replace_segments_with_compacted_segment_in_locked_scope()`:

1. Reacquires the flush-scope lock.
2. Reloads the current manifest.
3. Verifies the selected segments are still the exact trailing suffix with unchanged path, `_seq`
   range, row count, schema version, size, and status.
4. Truncates that suffix.
5. Appends the replacement compacted segment, or appends nothing if compaction proved every row in
   the suffix could be safely pruned.
6. Persists the updated `manifest.json` and refreshes hot tiers.

If a newer flush changed the suffix while compaction was writing, the swap is skipped, the new
compacted file is deleted, and the old manifest remains authoritative.

Compaction file names use `compact-<uuid>.parquet`, so they do not consume a new `batch-N`
sequence slot. `last_sequence_number` therefore remains the batch-file counter for future flushes.

Only after the manifest swap succeeds are the superseded source files deleted.

## Query path

Reads and planning go through `ManifestService`, not direct storage metadata reads.

For the full DataFusion provider scan path, MVCC merge algorithm, and PK fast paths, see
[hot-cold-storage-unification.md](hot-cold-storage-unification.md).

1. `ManifestAccessPlanner` requests the manifest for the relevant scope.
2. Segment pruning evaluates query predicates against persisted column stats.
3. DataFusion reads only the selected Parquet files.

The same manifest service also backs cold primary-key checks, so pruning and existence checks use
the same scope metadata the flush path maintains.

## Recovery and rebuild

Startup and recovery follow the same tier order:

- warm path: RocksDB manifest copy
- cold fallback: storage `manifest.json`
- rebuild fallback: scan Parquet files and reconstruct a manifest if storage metadata is missing

Shared manifests rehydrate memory on demand after RocksDB or storage reload. User manifests stay in
RocksDB only.

## Key components

| Component | Role | Location |
|-----------|------|----------|
| `ManifestService` | Canonical read-through/write-through manager for hot and cold manifest state | `backend/crates/kalamdb-flush/src/service.rs` |
| `FlushManifestHelper` | Flush-time helper for stats extraction, batch naming, and manifest commits | `backend/crates/kalamdb-flush/src/flush_helper.rs` |
| `FlushScopeWriter` | Atomic temp-write plus rename path for `batch-N.parquet` files | `backend/crates/kalamdb-flush/src/flush/scope_writer.rs` |
| `small_segment_compaction` | Tail-selection and MVCC-aware cold compaction implementation | `backend/crates/kalamdb-flush/src/compaction/small_segment.rs` |
| `CoreFlushScopeHook` | Core adapter for SQL plan cache invalidation and vector maintenance after a durable flush scope write | `backend/crates/kalamdb-core/src/manifest/mod.rs` |
| `Manifest`, `ManifestCacheEntry`, `SegmentMetadata` | Persisted manifest models and sync-state envelope | `backend/crates/kalamdb-system/src/providers/manifest/models/manifest.rs` |
