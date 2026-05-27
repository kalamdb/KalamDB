# Observability Views

KalamDB exposes low-overhead runtime observability through virtual system views instead of persisted DBA sampling tables.

## `system.stats`

`system.stats` is computed on demand as key/value rows. The assembly path now lives in `backend/crates/kalamdb-observability/src/system_stats.rs`, and `kalamdb-core` only provides an `AppContext` adapter that supplies domain counts and cache/storage snapshots.

The observability crate is now the source of truth for:

- runtime resource metrics
- query counters and latency
- manifest/parquet activity and flush counters
- open-file breakdowns used by health logging and `system.stats`

Query metrics are lock-free atomics on the hot path:

- `queries_total`
- `queries_per_second`
- `select_queries_total`
- `select_queries_per_second`
- `insert_queries_total`
- `insert_queries_per_second`
- `update_queries_total`
- `update_queries_per_second`
- `delete_queries_total`
- `delete_queries_per_second`
- `failed_queries_total`
- `avg_query_latency_ms`

The query rate is a bounded rolling window, and average latency is derived from accumulated execution time. Queries against `system.*` and `dba.*` tables are excluded from these counters so dashboard and administrative reads do not inflate user workload metrics. There is no background writer or persisted `dba` metrics table.

Storage-adjacent observability counters are also maintained as lightweight atomics in `kalamdb-observability` and updated from the owning write paths:

- `manifest_cache_rocksdb_entries`
- `manifest_cache_memory_entries`
- `manifest_reads_total`
- `manifest_reads_per_second`
- `manifest_writes_total`
- `manifest_writes_per_second`
- `flush_operations_total`
- `parquet_files_written_total`
- `parquet_files_written_per_second`
- `parquet_files_read_total`
- `parquet_files_read_per_second`
- `parquet_rows_flushed_total`

`ManifestService` initializes and maintains the manifest counters, records successful cold-store manifest reads and writes, the flush executor records successful parquet file writes after a flush completes, and `kalamdb-filestore` records successful parquet stream opens as read counters.

## `system.slow_queries`

Slow queries are written asynchronously to `slow.jsonl` after the configured threshold is exceeded. The `system.slow_queries` virtual view reads a bounded tail of that JSONL file and exposes recent entries with timestamp, duration, user, table metadata, row count, and redacted SQL text.

The view limits file IO to a fixed tail size and returns at most the most recent rows, so dashboard reads do not scan unbounded logs or add memory pressure while the server is idle.