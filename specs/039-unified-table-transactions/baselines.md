# Baselines: Unified Table Processing and Atomic Transaction Optimization

**Feature**: `039-unified-table-transactions`  
**Branch**: `feat/pg-dialect`  
**Profile**: `dev`  
**Hardware**: Apple M5 Pro, 15 cores, 24 GiB RAM, arm64, macOS 27.0.1  
**Recorded**: 2026-10-27, before provider edits

Pre-change rows are the comparison gate. Candidate rows are filled only after the change they measure. A blank candidate cell means that workload has not been re-run yet.

## Consolidation production lines

Counted by the quickstart script. A file's production lines stop at the first `#[cfg(test)]` module. File moves and deleted tests do not count as reduction.

| Root | Pre-change lines | Candidate lines |
|------|------------------|-----------------|
| `backend/crates/kalamdb-system/src/providers` | 13285 | 12540 |
| `backend/crates/kalamdb-system/src/macros.rs` | 434 | 190 |
| `backend/crates/kalamdb-tables/src` | 18269 | 17290 |
| `backend/crates/kalamdb-flush/src/flush` | 1429 | 1486 |
| `backend/crates/kalamdb-core/src/applier/executor/dml.rs` | 1852 | 1769 |
| `backend/crates/kalamdb-jobs/src/executors/flush.rs` | 642 | 636 |
| **Total** | **35911** | **33911** |

Candidate lines were counted on 2026-10-30 with the quickstart script. Net reduction is 2000 production lines. File moves and deleted tests are not included. The flush directory grew by 57 lines because publication locking moved into the versioned flush path; the other roots more than cover that.

## Gates 1–7

Recorded on 2026-10-30 from `cargo nextest run` on the default `dev` profile. Each `real` value is `/usr/bin/time -p` for that invocation and includes compilation. Every command exited 0. CLI smoke was not run: nothing serving KalamDB was listening on port 3000.

| Gate | Command | Real seconds |
|------|---------|--------------|
| 1 | `kalamdb-datafusion-sources` | 85.88 |
| 1 | `kalamdb-store` | 24.15 |
| 1 | `kalamdb-transactions` | 77.04 |
| 1 | `kalamdb-tables` | 42.03 |
| 1 | `kalamdb-publisher` | 139.35 |
| 1 | `kalamdb-plan-cache` | 77.09 |
| 1 | `kalamdb-transfer` | 66.17 |
| 2 | `kalamdb-system` | 70.80 |
| 2 | `kalamdb-views` | 88.20 |
| 2 | `kalamdb-core pg_catalog` | 610.79 |
| 2 | `kalamdb-tables --test provider_source_models` | 134.40 |
| 2 | `kalamdb-tables --test stream_provider_lightweight_scan` | 2.54 |
| 2 | `kalamdb-core covering_aggregate` | 0.70 |
| 2 | `kalamdb-core equality_update` | 0.58 |
| 3 | `kalamdb-core transaction` | 6.14 |
| 3 | `kalamdb-core applier` | 0.65 |
| 4 | `kalamdb-transactions overlay` | 5.12 |
| 5 | `kalamdb-flush` | 130.89 |
| 5 | `kalamdb-jobs` | 427.32 |
| 6 | `kalamdb-plan-cache` | 3.65 |
| 6 | `kalamdb-postgres-wire` | 459.40 |
| 6 | `kalamdb-core sql_executor` | 1.15 |
| 7 | `kalamdb-serialization` | 6.02 |

An earlier same-day measurement of the already compiled flush and jobs lib harnesses was 1.01s (51 passed) and 2.46s (72 passed). The Gate 5 rows above replace that as the official nextest timing. `mark_pending_write_does_not_wait_on_delayed_cold_flush_lock` and `idle_flush_scope_locks_return_to_steady_state_after_churn` passed inside the flush run.

## Workload timings

Columns follow T001. `batch submissions` applies to commit workloads. Lookup and introspection rows leave it blank. Median and p99 use nearest rank over five matched `dev` runs, so p99 is the slowest of the five. Times are the nextest-reported test duration after the harness is already compiled, in seconds.

| Workload | Build profile | Hardware note | Median seconds | p99 seconds | Batch submissions |
|----------|---------------|---------------|----------------|-------------|-------------------|
| Pre-change `system.users` primary-key lookup (`test_create_and_get_user`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Candidate `system.users` primary-key lookup (`test_create_and_get_user`) | dev | Apple M5 Pro, 24 GiB | 0.011 | 0.012 | |
| Pre-change `system.users` email index lookup (`test_get_active_oidc_invite_by_email_uses_email_index`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Candidate `system.users` email index lookup (`test_get_active_oidc_invite_by_email_uses_email_index`) | dev | Apple M5 Pro, 24 GiB | 0.011 | 0.012 | |
| Pre-change `system.jobs` primary-key lookup (`test_create_and_get_job`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Candidate `system.jobs` primary-key lookup (`test_create_and_get_job`) | dev | Apple M5 Pro, 24 GiB | 0.011 | 0.012 | |
| Pre-change namespace lookup (`test_create_and_get_namespace`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Candidate namespace lookup (`test_create_and_get_namespace`) | dev | Apple M5 Pro, 24 GiB | 0.011 | 0.011 | |
| Pre-change catalog type lookup (`implicit_row_type_and_alias_have_catalog_relationships`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Candidate catalog type lookup (`implicit_row_type_and_alias_have_catalog_relationships`) | dev | Apple M5 Pro, 24 GiB | 0.011 | 0.012 | |
| Pre-change `pg_catalog` namespaces, tables, and columns (`pg_catalog_shims_project_namespaces_tables_columns_and_database`, `kalamdb-core` `--test pg18`) | dev | Apple M5 Pro, 24 GiB | 0.080 | 0.112 | |
| Candidate `pg_catalog` namespaces, tables, and columns (`pg_catalog_shims_project_namespaces_tables_columns_and_database`, `kalamdb-core` `--test pg18`) | dev | Apple M5 Pro, 24 GiB | 0.074 | 0.078 | |
| Pre-change `information_schema.columns` introspection (`pg_catalog_shim_beekeeper_information_schema_column_query`, `kalamdb-core` `--test pg18`) | dev | Apple M5 Pro, 24 GiB | 0.085 | 0.115 | |
| Candidate `information_schema.columns` introspection (`pg_catalog_shim_beekeeper_information_schema_column_query`, `kalamdb-core` `--test pg18`) | dev | Apple M5 Pro, 24 GiB | 0.081 | 0.085 | |
| Pre-change 25,000-row insert | dev | Apple M5 Pro, 24 GiB | | | |
| Pre-change 25,000-row update | dev | Apple M5 Pro, 24 GiB | | | |
| Pre-change 25,000-row delete | dev | Apple M5 Pro, 24 GiB | | | |
| Candidate 25,000-row insert, update, and delete | dev | Apple M5 Pro, 24 GiB | | | |
| Candidate one-key fold of 25,000 updates (`fold_expands_primary_key_rename_and_collapses_one_key`, `kalamdb-transactions`) | dev | Apple M5 Pro, 24 GiB | 0.110 | 0.121 | |
| Candidate dirty reads of 100 and 1,000 rows (`dirty_reads_match_for_one_hundred_and_one_thousand_rows`, `kalamdb-transactions`) | dev | Apple M5 Pro, 24 GiB | 0.012 | 0.068 | |

The 25,000-row insert, update, and delete rows stay empty until Gate 3. Catalog candidate rows were measured on 2026-10-29 after the harness was already compiled: five `dev` nextest runs, nearest-rank median, p99 = slowest of the five. None exceeded its baseline.

T038 rows were measured on 2026-10-30 after the harness was already compiled: five `dev` nextest runs, nearest-rank median, p99 = slowest of the five. One-key fold samples: 0.121, 0.110, 0.110, 0.110, 0.109. Dirty-read samples: 0.068, 0.012, 0.012, 0.012, 0.012. These are unit-test durations. They are not the Gate 3 storage workload, and the tests do not assert the times.

## Performance handoff evidence

Source: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md), tasks T069–T089. One row per completed slice. The saved 2026-10-27 `benchv2/pgwire-speed` release totals are not this baseline. Do not compare `dev` timings to those release timings.

Timing run: `benchv2/pgwire-speed/results/runs/20261030T230656Z-dev-078ab01fc101`. `dev` profile, `KALAMDB_DML_STAGE_TRACE` unset, one discarded warm-up suite and five recorded suites. Median and range are in `summary.txt`, in seconds. KalamDB total median 35.734s, range 35.592..36.102. PostgreSQL total median 1.096s, range 1.092..1.134. Every recorded suite reported matching columns, row counts, `count(*)`, and returned-cell digests. This is the current baseline.

Profile run: `benchv2/pgwire-speed/results/runs/20261030T231604Z-dev-078ab01fc101` with `KALAMDB_DML_STAGE_TRACE=1`. One suite, not part of the timing median. Stage lines are in `kalam-dml-stage.log`. Nested stages are not subtracted. `scan_filter` on a storage iterator is omitted while a `collect_dataframe` scan guard is already active.

### Call path

| Stage or counter | Call |
|------------------|------|
| `planning` | `dataframe_from_statement` and `optimized_plan_for_cache` |
| `scan_filter` | `collect_dataframe`, or a forward RocksDB scan iterator when no collect guard is active |
| `overlay` | `merge_record_batches` after child partitions are read |
| `staging_fold` | `write_set.fold_commit_owned` on the commit path |
| `target_resolution` | `prepare_mixed_versioned_commit`, including row construction inside that function |
| `row_encode` | `VersionedRowCodec::encode` |
| `backend_submit` | RocksDB `batch` after coalescing returns operations. `units` is the operation count. `submitted_bytes` is key plus value length, not WAL file growth |
| `post_commit` | commit finishes, file cleanup, and deferred observe after the storage batch |
| `backend_lookups`, `fetched_bytes` | RocksDB `get`, `get_many`, and forward scan rows |
| `decoded_rows`, `decoded_bytes` | `VersionedRowCodec::decode` |
| `index_entries` | `scan_index_raw_typed_iter` and `scan_index_keys_iter` |
| `target_misses` | first insert of a key into `TargetResolutionCache` |
| `baseline_decodes` | one `load_mutation_target`, plus a full tombstone payload read in the hot insert scan. Counted only while `KALAMDB_DML_STAGE_TRACE` is on |

`row_encode`, `decoded_rows`, and `index_entries` stayed at zero for every script in the profile suite. Those functions are not the hot path of these 16 scripts. Reverse scans are not counted.

### Scripts 8, 11, 12, 13, and 15

Times are the profile suite, in milliseconds. The timing-run medians for the same scripts are in the table below and are not these profile samples.

| Script | Timing median seconds | Largest timed stage | Other timed stages | Counters |
|--------|----------------------:|---------------------|--------------------|----------|
| 8 UPDATE without an index | 3.829 | `scan_filter` 2055ms, 1008 spans | overlay 11, target resolution 16, backend submit 2, post-commit 3, fold 1 | 1,001,000 lookups, 109,977,868 fetched bytes, 1,000 target misses, 4,002 submitted operations |
| 11 INSERT from SELECT | 3.165 | `target_resolution` 402ms | fold 122, backend submit 122, post-commit 170, overlay 81, scan 61 | 51,000 target misses, 30,201,160 submitted bytes, 26,000 lookups |
| 12 DELETE without an index | 0.629 | `target_resolution` 157ms | scan 92, backend submit 57, post-commit 29, fold 12 | 9,642 target misses, 70,284 lookups, 7,744,566 fetched bytes |
| 13 DELETE with an index | 2.080 | `target_resolution` 538ms | backend submit 234, scan 112, post-commit 100, fold 42 | 32,460 target misses, 129,844 lookups, 194,762 submitted operations |
| 15 DELETE then many small INSERTs | 2.540 | `target_resolution` 416ms | post-commit 120, backend submit 89, fold 62, scan 35 | 38,000 target misses, 78,000 lookups, 16,218,421 submitted bytes |

Script 8's scan is the large stage. Scripts 11, 12, 13, and 15 spend more of their timed stages in target resolution and backend submit than in overlay or fold. Unexplained wall time is not assigned to a stage.

| Slice | Finding | Change | Measured outcome | Remaining issue |
|-------|---------|--------|------------------|-----------------|
| T069 | Dev baseline total median is 35.734s. Script 8 is scan-bound. Scripts 11, 12, 13, and 15 are dominated by target resolution, then backend submit. | Default-off `KALAMDB_DML_STAGE_TRACE` counters and a dev runner that keeps five suites in a new directory. | Timing digests matched on all five suites. Profile lines are in the run above. | `row_encode`, row decode, and index-iterator counters did not fire on this suite. |
| T070 | Inactive `buffer_put` and `buffer_delete` copied the key and value before noticing that no coalesce session was active. | Both helpers return before `to_vec` unless `coalesce_session_active` is true. Direct `batch` still returns the caller's operations. | Dev profile, one sample: 10,000 inactive put+delete pairs took 459µs and made 0 key or value copies. 10,000 active puts took 2.44ms and made 20,000 payload copies (one key and one value each). Existing coalesce, hook, and write-through tests passed. | The direct RocksDB write after the helper returns still allocates its own batch. |
| T071 | Commit held the original write set, its overlay, and a cloned folded payload across the Raft await. | `fold_commit_owned` clears overlay and latest-key maps, moves payloads into the folded result, and the coordinator drops that result's admission history before the await. A net-no-op fold keeps payload-free history plus the original byte count for group admission. | Dev profile, one sample. 25,000 distinct inserts: borrowed fold retained history and fold together at 4,377,780 bytes; owned fold retains 2,188,890. 25,000 updates of one key: history 2,075,000 bytes, owned fold 83. Insert-then-delete keeps 2 admission identities and the original 178 accounted bytes. Existing fold transition tests passed. | Savepoint history is still cloned by `fold_commit` while the transaction is open. The owned path runs only after commit has sealed the transaction. |
| T072 | Mixed-commit preparation decoded each target inside the per-row plan, and file cleanup decoded it again. Mixed inserts decoded the hot row in `ensure_insert_pk_unique` and left a tombstone in place. | Prefetch loads each distinct `ScopedTargetKey` once, in table-and-scope batches of 64, into a cache that keeps the storage baseline separate from a row this commit already wrote. Update and delete planning receive that baseline and do not load again. File refs come from the cached live row. Auto-increment insert-only keys skip the load. A hot tombstone is the insert predecessor. A live hot or cold row is a primary-key violation. Unwatched updates move the prior row into the replacement. Commit finish clones fields only when the table has vector columns. `check_expected_max_version` is not called with `None`. | Dev profile. `cargo nextest run -p kalamdb-tables --lib versioned::target`: 4 passed in 0.017s. 65 user keys requested twice, plus one shared key requested twice, become 3 batches (64, 1, 1) and 66 loads. The user batches are not insert-only. The shared batch is. `missing_snapshot_bound_is_not_validation` passed. `cargo check -p kalamdb-core --lib` passed. No new timing median. | `baseline_decodes` increments only while stage trace is on: once per `load_mutation_target`, and once when the hot insert scan loads a tombstone payload. A deleted-flag prefix scan is not counted. A liveness recheck after a claim wait is a conflict retry and is outside this budget. Range DELETE already loaded once per key. The first cache store, including a resolved miss, still increments `target_misses`. A cold primary-key error from this path no longer includes the segment path. |
| T073 | Each mixed-commit row marked the manifest pending and, for inserts, built a notification row even when nothing was watching. | Upserts and deletes in one commit share one scope finish per table and user. That finish marks pending write once. Vector rows are staged only when the table has vector columns, in commit order. Unwatched insert and update notifications are not built. File cleanup still uses the cached prior row. Scope finishes run only after `StorageBackend::batch` returns `Ok`. | Dev profile. `cargo nextest run -p kalamdb-core --lib one_scope_group_per_table_and_user`: 1 passed in 0.017s. Five same-provider events for two users, plus two shared events, become 3 scope groups. The unwatched group keeps an empty op list. `cargo check -p kalamdb-core --lib` passed. No new timing median. | All-insert batches still use their existing one-mark finish. Autocommit range delete and update already marked once per batch. A failed batch returns before scope finishes; there is no separate injected-failure test. Reported `manifest_updates` still counts rows. |
| T075 | Staging checked one transaction's buffer and did not bound the sum across transactions, or release that charge when the transaction ended. | `MemoryLedger` reserves staged payload, the overlay copy, scan batches, the target cache, encoded commit batches, and side effects with atomics. Process caps are `max_transaction_buffer_bytes * 8`, except scan batches, which use the DataFusion memory limit. A reservation is taken before the bytes are retained. Commit, rollback, timeout, abort, and savepoint rollback release it. Encoded commit bytes are reserved before prepare and released when that scope ends. RocksDB cache and RSS are not counted. | Dev profile. `cargo nextest run -p kalamdb-commons --lib memory_ledger`: 4 passed in 0.013s. A charge at the limit rejects the next byte and releases back to zero. A staged charge that cannot also cover the overlay copy reserves neither. `cargo check -p kalamdb-core --lib` passed. No new timing median. | T081 still owns the adversarial wide-row, nested-value, and concurrent-transaction resource tests. |
| T076 | UPDATE and DELETE collected every matching Arrow batch, decoded it to rows, then encoded assignment chunks again. INSERT SELECT held the whole input plan before decoding. | Scan and INSERT SELECT input are visited one batch at a time. Assignments evaluate on that batch. Direct writes run after the scan returns, so the statement does not see its own new rows. Pieces stay within `batch_size` and the DataFusion memory-pool byte limit. One row over that limit is `ResourcesExhausted`. | Dev profile. `cargo nextest run -p kalamdb-datafusion-sources -p kalamdb-tables --lib one_wide_row multi_row_batch prepared_assignments_match dml_scan_projection`: 5 passed in 0.025s. A 64-byte text row fails a 16-byte limit. Four 4-byte rows split under a 6-byte limit. `evaluate_batch` matches row evaluation. Projection tests still omit unused columns. | Scripts #8, #11, and #12 not re-timed (dirty-tree baseline forbidden). Statement write set still retained. |
| T078 | Hot scans resolved into one batch; overlay could not drop chunks. | Columnar hot scan resolves the range, then emits session `batch_size` chunks via `DeferredBatchOutput::follow_batches`. `DeferredBatchExec` streams them. Row-map / index / cold paths also emit session-sized batches after resolution. | Dev profile. Existing deferred/overlay/hot-scan suites remain green. No new script #4/#5 median. | Winner map and scalar columns still span the scan. Index/cold still collect before split. Scripts #4 and #5 not re-measured. |
| T081 | Scan collect and nested scalars were uncharged; concurrent staged admission lacked unit coverage. | `Reservation::adopt`, `QueryBatchStream::collect_accounted`, `ExecutionResult::into_materialized_rows_accounted`. Nested List/Struct/Map scalars use Arrow `get_array_memory_size`. Concurrent staged admission and wide-row bound tests. | Dev profile. `cargo nextest run -p kalamdb-commons --lib memory_ledger`: 6 passed in 0.014s. `cargo nextest run -p kalamdb-core --lib accounted_collect`: 1 passed in 0.019s. | Large-DML peak tracked vs RSS awaits T089 clean-tree run. |
| T085 / T051 | Concurrent cold plan misses each rebuilt the same logical plan. | `try_lead` / `finish_lead` / `cancel_lead` singleflight with Notify wakeup. SELECT and DML join one build, then bind in the caller. Catalog-function plans cancel instead of publishing. | Dev profile. Existing `kalamdb-plan-cache` lead/cancel/follower tests remain green. | Peak planning bytes, planning CPU, and p95 latency not measured on a clean tree. |
| T086 | Cold Parquet scans repeatedly copied a growing prefix when concatenating batches. | `ColdBatchRead` delivers one unread batch with backpressure. Count and PK cold paths fold without whole-prefix copies. Bounded file-open concurrency (8). | Dev profile. Cold planner APIs compile; concat helpers remain for transitional collect. No copied-byte median. | Copied bytes, peak memory, and seconds not measured. |
| T087 | SELECT materialised the full DataFusion result before wire/HTTP encoding. | `ExecutionResult::StreamingRows` + `QueryBatchStream` (buffer=1). Wire async encoder pulls batches. HTTP/cluster/functions/live call `into_materialized_rows`. Accounted collect adapter for ScanBatch. | Dev profile. `accounted_collect_releases_scan_bytes_on_drop` passed (0.019s). `cargo check -p kalamdb-core --lib` passed. | Large SELECT first-row/total/peak-bytes and throttled-client runs await clean-tree measurement. |
