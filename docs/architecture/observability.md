# Observability Views

KalamDB exposes low-overhead runtime observability through virtual system views instead of persisted DBA sampling tables.

## `system.stats`

`system.stats` is computed on demand as key/value rows. The assembly path now lives in `backend/crates/kalamdb-observability/src/system_stats.rs`, and `kalamdb-core` only provides an `AppContext` adapter that supplies domain counts and cache/storage snapshots.

The observability crate is now the source of truth for:

- runtime resource metrics
- query counters and latency
- manifest/parquet activity and flush counters
- open-file breakdowns used by health logging and `system.stats`

## Traceability feature gate

Runtime metrics and dashboard counters are separate from tracing spans. The admin UI relies on
`system.stats`, so metrics remain enabled by default through `kalamdb-observability`'s `metrics`
feature and are still compiled when traceability is disabled.

Tracing spans and trace/debug events that sit on query, RocksDB, row serialization, DML collection,
manifest, and Parquet hot paths must go through the `kalamdb-observability` macro facade instead of
calling `tracing::*` directly. The server's default feature set includes `traceability` for current
developer behavior. Production builds that want dashboard metrics without hot-path tracing can use:

```bash
cd backend && cargo build --release --no-default-features --features embedded-ui,mimalloc
```

The macros compile to no-ops without evaluating span fields or timing expressions when
`traceability` is not enabled. This keeps the observability seam deep: callers express the span/event
once, while the crate controls whether it becomes a `tracing` span or disappears at compile time.

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

Pub/sub and live-subscription metrics are also lock-free atomics or in-memory cache counts; dashboard reads do not scan topic message storage:

- `pubsub_messages_published_total`
- `pubsub_messages_published_per_second`
- `pubsub_messages_published_peak_per_second`
- `pubsub_bytes_published_total`
- `pubsub_kb_published_per_second`
- `pubsub_messages_consumed_total`
- `pubsub_messages_consumed_per_second`
- `pubsub_messages_consumed_peak_per_second`
- `pubsub_bytes_consumed_total`
- `pubsub_kb_consumed_per_second`
- `pubsub_active_consumers`
- `pubsub_active_consumers_peak`
- `subscription_changes_delivered_total`
- `subscription_changes_delivered_per_second`
- `topic_consumer_group_count`
- `topic_consumer_partition_count`

Publish and consume counters are recorded only after successful topic writes or reads. Active consumer counts wrap HTTP and SQL consume requests, while consumer group totals come from the topic publisher's runtime state and restored offset metadata.

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
