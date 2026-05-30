# ADR-006: Flush Execution and Post-Flush Tail Compaction

**Status**: Accepted  
**Date**: 2025-10-22  
**Updated**: 2026-05-24
**Related**: ADR-001 (Table-per-User), ADR-005 (RocksDB Metadata Only), ADR-007 (Storage Registry)

## Context

KalamDB uses a write-hot / flush-cold architecture:

- hot rows live in RocksDB-backed table stores
- cold rows live in immutable Parquet segments
- manifests track cold segments per table scope and are owned by `ManifestService`

The current implementation must satisfy these constraints:

1. Keep the write path cheap by avoiding cold-storage metadata rewrites on every DML statement.
2. Flush large scopes in bounded batches instead of materializing entire tables in memory.
3. Preserve user-scope isolation for `USER` tables while keeping `SHARED` tables as one flush scope.
4. Make Parquet writes and manifest commits crash-safe.
5. Preserve latest tombstones until cold compaction proves they are no longer needed.
6. Compact only the unstable small-file tail instead of repeatedly rewriting older cold segments.

## Decision

### 1. Flush is scope-based and batched

Flush jobs operate on manifest scopes with pending writes.

- `USER` flush resolves one `(table_id, user_id)` scope at a time.
- `SHARED` flush resolves one table-wide shared scope.
- Hot rows are scanned in bounded batches controlled by `flush.flush_batch_size`.

Version resolution keeps only the latest `_seq` per primary-key value. When a row has no primary
key value, flush falls back to `_seq:<value>` so version resolution remains deterministic.

### 2. Flush keeps latest tombstones hot

The current flush path does not write tombstones to Parquet. Instead:

- latest delete markers are filtered out of the cold output
- the winning tombstone key remains in RocksDB
- older versions can therefore stay masked until a later cold compaction inspects older segments

This keeps the cold write path simple while preserving correct MVCC visibility.

### 3. Cold writes are atomic per scope

`FlushScopeWriter` performs the cold write under the manifest flush-scope lock:

1. Mark the manifest as `syncing`.
2. Allocate the next `batch-N.parquet` name from `manifest.last_sequence_number`.
3. Write `batch-N.parquet.tmp`.
4. Atomically rename it to `batch-N.parquet`.
5. Compute `_seq` range, row count, schema version, size, and indexed-column stats.

This keeps batch numbering serialized per scope and prevents overlapping flushes from racing the
manifest append.

### 4. Flush commits the manifest through one canonical path

After the Parquet rename succeeds, flush commits segment metadata through
`ManifestService::persist_flushed_segment()`.

That path:

1. loads or initializes the manifest for the scope
2. appends the new `SegmentMetadata`
3. writes storage `manifest.json`
4. refreshes the RocksDB manifest copy
5. refreshes shared-scope memory cache when applicable

Flush therefore treats `manifest.json` persistence as part of the successful cold commit, not as a
separate background best-effort step.

### 5. Hot-row cleanup happens only after the cold commit

Hot rows are deleted from RocksDB only after both of these succeed:

- the Parquet batch rename to its final filename
- the manifest append and persistence

Cleanup is performed in bounded delete batches. If a flush fails before that point, rows remain hot
and can be retried.

### 6. Post-flush small-segment compaction is optional and leader-only

KalamDB now supports optional post-flush compaction under `[flush.compaction]`.

Defaults:

```toml
[flush.compaction]
enabled = false
min_eligible_segments = 5
max_segments_per_run = 8
user_max_segment_rows = 10000
shared_max_segment_rows = 25000
```

Flush does not compact old Parquet files inline. Instead, successful flushes emit scope hints, and
the jobs layer may enqueue a leader-only `segment_compact` job for eligible scopes.

### 7. Tail compaction rewrites only the trailing small-file run

Compaction selects from the newest end of the manifest and stops at the first segment that is:

- already at or above the target row count
- unreadable / not committed
- on a different schema version

This keeps compaction focused on the unstable tail instead of rewriting already-sized historical
segments.

### 8. Tail compaction is MVCC-aware and manifest-safe

Compaction runs in two phases:

1. Read the selected tail and determine the latest version for each key using primary key, `_seq`,
   and `_deleted`.
2. Inspect older segments to decide which delete tombstones must be preserved to continue masking
   older cold rows.
3. Stream only winning rows into `compact-<uuid>.parquet.tmp`.
4. Rename to `compact-<uuid>.parquet`.

The old manifest remains readable during this work.

### 9. Manifest compaction is suffix replacement

The compacted Parquet file becomes visible only if the selected input segments are still the exact
trailing suffix of the manifest when the scope lock is reacquired.

`replace_segments_with_compacted_segment_in_locked_scope()` then:

1. verifies the suffix by path, `_seq` range, row count, size, schema version, and status
2. truncates the suffix
3. appends the replacement compacted segment, or appends nothing if the suffix was fully pruned
4. persists the updated `manifest.json`
5. refreshes the hot manifest tiers

If the suffix changed while compaction was writing, the swap is aborted and the new compacted file
is deleted.

Compaction filenames use `compact-<uuid>.parquet`, so they do not consume a new `batch-N` slot.

### 10. Old compacted inputs are deleted only after the manifest swap

Source files are deleted only after the manifest points at the replacement file, or after the
suffix has been removed entirely. If the manifest swap does not happen, existing files remain the
authoritative source of truth.

## Consequences

### Positive

1. The write path stays cheap because manifests are marked dirty hot-first and persisted on flush.
2. Flush memory stays bounded by scan batch size and active scope, not table size.
3. User isolation is preserved because user tables flush and compact per user scope.
4. Parquet + manifest commit is atomic at the scope level.
5. Tail compaction reduces small-file buildup without blocking normal flushes.
6. MVCC visibility remains correct across hot rows, cold rows, and delete tombstones.

### Trade-offs

1. Latest tombstones may remain hot for some time until cold compaction can safely remove them.
2. User-heavy workloads can still create many small cold segments before compaction converges.
3. Compaction may be skipped if a newer flush changed the manifest tail during rewrite work.

## Notes for readers

The detailed implementation lives in:

- `backend/crates/kalamdb-flush/src/flush/`
- `backend/crates/kalamdb-flush/src/flush_helper.rs`
- `backend/crates/kalamdb-flush/src/service.rs`
- `backend/crates/kalamdb-flush/src/compaction/small_segment.rs`
- `backend/crates/kalamdb-core/src/manifest/mod.rs` for core-specific adapters around the flush crate

For the current operational description, prefer [docs/architecture/manifest.md](../manifest.md).

```rust
fn flush_user_data(&self, user_id: &str, rows: &[(Vec<u8>, JsonValue)]) -> Result<usize, KalamDbError> {
    // T161c: Resolve storage path using template
    let user_storage_path = self.resolve_storage_path_for_user(&user_id)?;
    
    // T161b: Generate ISO 8601 filename
    let batch_filename = self.generate_batch_filename(); // "2025-10-22T14-30-45.parquet"
    let output_path = PathBuf::from(&user_storage_path).join(&batch_filename);
    
    // Write Parquet file
    let writer = ParquetWriter::new(output_path.to_str().unwrap());
    writer.write(self.schema.clone(), vec![batch])?;
    
    Ok(rows.len())
}
```

### Template Resolution (T161c)

```rust
fn resolve_storage_path_for_user(&self, user_id: &UserId) -> Result<String, KalamDbError> {
    if let Some(ref registry) = self.storage_registry {
        let storage = registry.get_storage("local")?;
        let template = &storage.user_tables_template;
        
        // Single-pass substitution
        let path = template
            .replace("{namespace}", self.namespace_id.as_str())
            .replace("{tableName}", self.table_name.as_str())
            .replace("{userId}", user_id.as_str())
            .replace("{shard}", ""); // Future: sharding strategy
        
        let full_path = format!("{}/{}", storage.base_directory, path);
        Ok(full_path)
    } else {
        // Fallback to legacy path
        Ok(self.substitute_user_id_in_path(user_id))
    }
}
```

## References

- **ADR-001**: Table-per-User Architecture (explains why per-user isolation matters)
- **ADR-005**: RocksDB Metadata Only (explains write-to-buffer + flush-to-Parquet pattern)
- **ADR-007**: Storage Registry (explains template validation and multi-storage support)
- **T151-T151h**: Streaming flush implementation tasks
- **T161a-T161c**: Per-user file isolation and template resolution tasks
- **T162-T162b**: Documentation tasks for this ADR

## Future Enhancements

1. **Sharding Support** (User Story 6): Populate `{shard}` variable using sharding strategy
2. **Compaction scheduling**: Add time/manual triggers if post-flush-only compaction is not enough
3. **Parallel Flush**: Process multiple users concurrently (requires thread-safe RocksDB iterator)
4. **Incremental Flush**: Track last flush timestamp per user, only flush new rows
