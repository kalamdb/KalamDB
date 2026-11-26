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
| T076 | UPDATE and DELETE collected every matching Arrow batch, decoded it to rows, then encoded assignment chunks again. INSERT SELECT held the whole input plan before decoding. | Scan and INSERT SELECT input are visited one batch at a time. Assignments evaluate on that batch. Direct writes run after the scan returns, so the statement does not see its own new rows. Pieces stay within `batch_size` and the DataFusion memory-pool byte limit. One row over that limit is `ResourcesExhausted`. | Dev profile. `cargo nextest run -p kalamdb-datafusion-sources -p kalamdb-tables --lib one_wide_row multi_row_batch prepared_assignments_match dml_scan_projection`: 5 passed in 0.025s. A 64-byte text row fails a 16-byte limit. Four 4-byte rows split under a 6-byte limit. `evaluate_batch` matches row evaluation. Projection tests still omit unused columns. | Re-timed on 2026-11-15: #8 3.744s, #11 2.952s, #12 0.674s. Statement write set still retained. |
| T078 | Hot scans resolved into one batch; overlay could not drop chunks. | Columnar hot scan resolves the range, then emits session `batch_size` chunks via `DeferredBatchOutput::follow_batches`. `DeferredBatchExec` streams them. Row-map / index / cold paths also emit session-sized batches after resolution. | Dev profile. Existing deferred/overlay/hot-scan suites remain green. No new script #4/#5 median. | Winner map and scalar columns still span the scan. Index/cold still collect before split. Scripts #4 and #5 were re-measured on 2026-11-15 at 0.459s and 0.647s. |
| T081 | Scan collect and nested scalars were uncharged; concurrent staged admission lacked unit coverage. | `Reservation::adopt`, `QueryBatchStream::collect_accounted`, `ExecutionResult::into_materialized_rows_accounted`. Nested List/Struct/Map scalars use Arrow `get_array_memory_size`. Concurrent staged admission and wide-row bound tests. | Dev profile. `cargo nextest run -p kalamdb-commons --lib memory_ledger`: 6 passed in 0.014s. `cargo nextest run -p kalamdb-core --lib accounted_collect`: 1 passed in 0.019s. | Large-DML peak tracked vs RSS awaits T089 clean-tree run. |
| T085 / T051 | Concurrent cold plan misses each rebuilt the same logical plan. | `try_lead` / `finish_lead` / `cancel_lead` singleflight with Notify wakeup. SELECT and DML join one build, then bind in the caller. Catalog-function plans cancel instead of publishing. | Dev profile. Existing `kalamdb-plan-cache` lead/cancel/follower tests remain green. | Peak planning bytes, planning CPU, and p95 latency not measured on a clean tree. |
| T086 | Cold Parquet scans repeatedly copied a growing prefix when concatenating batches. | `ColdBatchRead` delivers one unread batch with backpressure. Count and PK cold paths fold without whole-prefix copies. Bounded file-open concurrency (8). | Dev profile. Cold planner APIs compile; concat helpers remain for transitional collect. No copied-byte median. | Copied bytes, peak memory, and seconds not measured. |
| T087 | SELECT materialised the full DataFusion result before wire/HTTP encoding. | `ExecutionResult::StreamingRows` + `QueryBatchStream` (buffer=1). Wire async encoder pulls batches. HTTP/cluster/functions/live call `into_materialized_rows`. Accounted collect adapter for ScanBatch. | Dev profile. `accounted_collect_releases_scan_bytes_on_drop` passed (0.019s). `cargo check -p kalamdb-core --lib` passed. | Large SELECT first-row/total/peak-bytes and throttled-client runs await a throttled-client measurement. |
| T088 | The shipped A–F slices had unit coverage, and the full CLI suite had not been recorded against this tree. | Re-ran the quickstart gate crates and the filtered core catalog/transaction/applier/executor set, and used the same-day full CLI e2e. | Dev profile, Apple M5 Pro, commit `90fa854bdc01`. Gate crates: 842 passed, 0 failed, 2.456s (real 521.18s including the dependency rebuild). Core filter: 111 passed, 0 failed, 5.653s (real 605.19s including compile). CLI e2e: 677 passed, 8 skipped, 0 failed, 378.631s. | File, vector, restart, and cancellation cases are the ones already inside those suites. |
| T089 | October dev total median was 35.734s. Combined scripts 8, 11, 12, 13, and 15 were 12.243s. | Clean-tree `KALAM_BASELINE=1` five-suite run, then one `KALAMDB_DML_STAGE_TRACE=1` suite. | Dev profile, Apple M5 Pro, `90fa854bdc01`. Timing run `benchv2/pgwire-speed/results/runs/20261115T202931Z-dev-90fa854bdc01`. KalamDB total median 26.458s, range 26.197..26.564. PostgreSQL total median 1.084s, range 1.077..1.089. Every recorded suite matched columns, row counts, `count(*)`, and cell digests. Profile run `20261115T203401Z-dev-90fa854bdc01` is one sample and is not part of the median. | Combined scripts 8/11/12/13/15 are 11.811s, 3.5% under 12.243s. The 30% combined goal is unmet. Script 12 is slower (0.629s to 0.674s). Peak tracked bytes, RSS, 1/8/32-client prepared CRUD, and the 300-second churn series are in the 2026-11-17 section below. |

## T089 candidate timing, 2026-11-15

The October 30 run stays the baseline. This candidate is the same `dev` protocol: one discarded warm-up suite and five recorded suites, `KALAMDB_DML_STAGE_TRACE` unset. Median and range are in seconds.

| Script | October 30 median | November 15 median | November 15 range |
|--------|------------------:|-------------------:|-------------------|
| 1 | 0.129 | 0.133 | 0.132..0.137 |
| 2 | 2.526 | 2.493 | 2.483..2.517 |
| 3 | 2.595 | 2.566 | 2.511..2.579 |
| 4 | 3.959 | 0.459 | 0.418..0.471 |
| 5 | 5.815 | 0.647 | 0.617..0.687 |
| 6 | 0.328 | 0.340 | 0.334..0.342 |
| 7 | 0.611 | 0.616 | 0.607..0.632 |
| 8 | 3.829 | 3.744 | 3.595..3.770 |
| 9 | 2.884 | 2.832 | 2.805..2.838 |
| 10 | 3.050 | 2.992 | 2.980..3.002 |
| 11 | 3.165 | 2.952 | 2.934..2.966 |
| 12 | 0.629 | 0.674 | 0.669..0.679 |
| 13 | 2.080 | 1.910 | 1.904..1.938 |
| 14 | 1.536 | 1.508 | 1.505..1.516 |
| 15 | 2.540 | 2.531 | 2.525..2.536 |
| 16 | 0.064 | 0.062 | 0.061..0.070 |
| total | 35.734 | 26.458 | 26.197..26.564 |

The total is 9.276s lower, 26.0%. Scripts 4 and 5 account for 8.668s of that. Script 7, the indexed point-select suite, moved from 0.611s to 0.616s. That is under the flag line of max(5%, 0.005s). Script 12 moved from 0.629s (0.624..0.642) to 0.674s (0.669..0.679). The ranges do not overlap, and 0.045s is above max(5% of 0.629s, 0.005s). Combined scripts 8, 11, 12, 13, and 15 are 11.811s, 0.432s under 12.243s, which is 3.5%.

Profile suite stage lines, one sample, nested stage time included in full. Stage sums can exceed wall time.

| Script | Wall in the profile suite | Largest named stages |
|--------|--------------------------:|----------------------|
| 4 | 0.469s | `scan_filter` 1765ms across 116 spans, 2,271,393 lookups |
| 5 | 0.645s | `scan_filter` 6154ms across 155 spans, `planning` 220ms |
| 8 | 3.650s | `scan_filter` 2008ms, 1,001,000 lookups, 109,977,868 fetched bytes |
| 11 | 2.887s | `target_resolution` 406ms, `backend_submit` 131ms, `staging_fold` 101ms |
| 12 | 0.667s | `target_resolution` 172ms, `scan_filter` 91ms, `backend_submit` 59ms |
| 13 | 1.874s | `target_resolution` 590ms, `backend_submit` 184ms |
| 15 | 2.492s | `target_resolution` 499ms, `backend_submit` 92ms, `staging_fold` 60ms |

Script 8 is still scan-bound. Scripts 11, 12, 13, and 15 still spend their largest named stage in target resolution. The 30% combined-script goal remains unmet. Peak tracked bytes, RSS, prepared CRUD, and the churn series are recorded below.

## OpenRaft 0.9.25 baseline (T090)

Recorded 2026-11-15 from the tree that still pins OpenRaft 0.9.25. No timings. This is the behavior T091–T097 must still satisfy.

| Item | Current behavior |
| --- | --- |
| Pin | `openraft = { version = "0.9.25", features = ["serde", "storage-v2"] }` in the workspace `Cargo.toml` |
| Network | Unary `RaftNetwork` in `backend/crates/kalamdb-raft/src/network/`. No `RaftNetworkV2`, no `full_snapshot` chunking |
| `system.cluster_groups.committed` | Hard-coded `None` in `backend/crates/kalamdb-views/src/system/cluster_groups.rs` (`committeds.push(None)`) |
| Leadership transfer | `RaftGroup::transfer_leadership` returns `InvalidState("unsupported")` in `backend/crates/kalamdb-raft/src/manager/raft_group.rs` |
| Heartbeat age | `millis_since_last_heartbeat: None` in `backend/crates/kalamdb-raft/src/executor/raft.rs` |
| Log I/O | Four workers (`LOG_IO_SHARDS = 4`) in `backend/crates/kalamdb-raft/src/storage/log_io.rs`. A full queue uses a blocking `mpsc::sync_channel` send (`LOG_IO_QUEUE_DEPTH = 64`) |
| Stored log ids | `stored_log_id()` rebuilds every disk log id with `CommittedLeaderId::new(term, 0)` in `raft_store.rs`. The log record stores only term and index |
| Snapshot retention config | Multi-node `Config` sets `max_in_snapshot_log_to_keep: config.max_snapshots_to_keep` in `raft_group.rs`. That field counts log entries, not snapshot files |

Fixtures written by `backend/crates/kalamdb-raft/tests/openraft_compat.rs` live in `backend/crates/kalamdb-raft/tests/fixtures/openraft-0.9/`: `vote.bin`, `membership.bin`, `command_entry.bin`, `state_machine_data.bin`, `snapshot.bin`, `raft_snapshot_meta.bin`, `kalam_node.bin`. Membership and the snapshot header use the state-machine msgpack envelope. The vote and `RaftSnapshotMeta` use `encode_entity`. The command entry is a packed log record (`index = 2`, `term = 1`, normal tag) around `RaftCommand::Meta(CreateNamespace)`. The follower-restart write test is `follower_restart_then_more_writes` in `tests/cluster_integration.rs`.

## Spec 039 regression on the 0.10 tree (T111)

Recorded 2026-11-16, `dev` profile, `--retries 0`. One nextest invocation. Summary time is the nextest total, not compile time. 28 passed, 0 failed.

```bash
cd backend
cargo nextest run -p kalamdb-core -p kalamdb-raft -p kalamdb-commons --offline --retries 0 -E 'test(/explicit_commit_persists_same_table_same_user_user_inserts|explicit_commit_preserves_user_scope_for_same_primary_keys|operation_service_preserves_user_scope_for_same_primary_keys|explicit_commit_of_25000_user_rows_is_all_or_nothing|explicit_commit_of_25000_shared_rows_is_all_or_nothing|interrupted_durable_commit_is_wholly_present_or_absent|replay_of_applied_commit_does_not_duplicate_rows|cross_domain_commit_is_rejected_before_a_storage_batch|sql_request_transaction_commit_persists_rows|sql_request_transaction_rejects_cross_group_access_without_aborting_bound_group|standalone_transaction_rejects_second_group_before_staging_and_can_commit_first|standalone_transaction_commits_two_users_in_its_single_user_group|one_node_cluster_transaction_rejects_a_different_shared_shard|test_user_data_command_watermark_get_set|test_all_user_command_variants_get_watermark|test_user_command_set_watermark_to_zero|test_shared_data_command_watermark|test_all_shared_command_variants_get_watermark|test_shared_command_set_watermark_to_zero|test_advance_and_check|test_wait_for_already_satisfied|test_add_and_drain|test_drain_preserves_log_order|assign_entry_versions_packs_log_index_and_ordinal|pack_unpack_round_trip_matches_numeric_order|ordinal_bounds_and_slot_count|log_index_bounds_stay_non_negative|test_version_ordering/)'
```

| Suite | Tests | Passed | Failed | Slowest |
| --- | ---: | ---: | ---: | --- |
| Single-group atomic commit (`kalamdb-core` transaction binaries) | 13 | 13 | 0 | `explicit_commit_of_25000_user_rows_is_all_or_nothing` 1.523s |
| Meta watermark (`kalamdb-raft` command, coordinator, and pending-buffer tests) | 10 | 10 | 0 | 0.019s |
| CDC `VersionId` order (`assign_entry_versions_packs_log_index_and_ordinal`, `pack_unpack_round_trip_matches_numeric_order`, `ordinal_bounds_and_slot_count`, `log_index_bounds_stay_non_negative`, `test_version_ordering`) | 5 | 5 | 0 | 0.018s |
| Total | 28 | 28 | 0 | summary 1.528s |

## Partial 25,000-row sample (not the T112 gate)

T090 recorded no 0.9 timings, so this is not a before/after comparison. The command is the existing all-or-nothing test: one successful 25,000-row user commit and one rejected duplicate commit. It is not a 1,000-row insert and not a 3-node group. Five `dev` runs, `--retries 0`, after the test binary was already built. CPU was not collected.

```bash
cd backend
cargo nextest run -p kalamdb-core --offline --retries 0 -E 'test(explicit_commit_of_25000_user_rows_is_all_or_nothing)'
```

| Run | Test seconds | Peak RSS |
| --- | ---: | ---: |
| 1 | 1.333 | compile included, 2,902,130,688 bytes |
| 2 | 1.294 | 281,870,336 bytes |
| 3 | 1.310 | 284,295,168 bytes |
| 4 | 1.327 | 280,641,536 bytes |
| 5 | 1.307 | 281,264,128 bytes |

Sorted test seconds: 1.294, 1.307, 1.310, 1.327, 1.333. Median 1.310s. With five samples the nearest-rank p95 and p99 are both 1.333s. Peak RSS of the four runs that did not compile is 284,295,168 bytes.

## OpenRaft 0.10 apply timings (T112)

Recorded 2026-11-17 on darwin 27.2.0, `dev` profile, `--retries 0`, `--offline`. Both binaries were already built. The timer wraps only `commit_transaction` after the rows are staged, or only the 100 Meta `CreateNamespace` proposals. Fifteen samples, none discarded. Percentiles are nearest rank, so with 15 samples p95 and p99 are the maximum sample.

Before is the committed 0.9.25 tree at `2b6d91e14`, compiled with Rust 1.99.1. After is the dirty 0.10.0 working tree. The same test source ran on both. CPU and peak RSS are `/usr/bin/time -l` on the `cargo nextest` process for that invocation.

```bash
# standalone inserts
T112_INSERT_REPORT=/tmp/t112-insert-before.txt cargo nextest run -p kalamdb-core --offline --retries 0 --test transactions -E 'test(t112_standalone_insert_commit_times)'
T112_INSERT_REPORT=/tmp/t112-insert-after.txt cargo nextest run -p kalamdb-core --offline --retries 0 --test transactions -E 'test(t112_standalone_insert_commit_times)'

# 3-node busy group
T112_BURST_REPORT=/tmp/t112-burst-before.txt T112_RPC_PORT=15900 cargo nextest run -p kalamdb-raft --offline --retries 0 --test t112_busy_group -E 'test(t112_three_node_busy_group)'
T112_BURST_REPORT=/tmp/t112-burst-after.txt T112_RPC_PORT=15700 cargo nextest run -p kalamdb-raft --offline --retries 0 --test t112_busy_group -E 'test(t112_three_node_busy_group)'
```

| Workload | Tree | min | p50 | p95 | p99 | max | nextest | user | sys | peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000-row commit | 0.9.25 | 0.032888 | 0.033129 | 0.045192 | 0.045192 | 0.045192 | 15.315s | 17.88s | 0.63s | 458,883,072 |
| 1,000-row commit | 0.10.0 | 0.033385 | 0.033810 | 0.045512 | 0.045512 | 0.045512 | 15.486s | 18.03s | 0.68s | 489,586,688 |
| 25,000-row commit | 0.9.25 | 0.833753 | 0.842913 | 0.850763 | 0.850763 | 0.850763 | 15.315s | 17.88s | 0.63s | 458,883,072 |
| 25,000-row commit | 0.10.0 | 0.839307 | 0.845035 | 0.866875 | 0.866875 | 0.866875 | 15.486s | 18.03s | 0.68s | 489,586,688 |
| 3-node, 100 Meta proposals | 0.9.25 | 0.062307 | 0.063006 | 0.090140 | 0.090140 | 0.090140 | 1.551s | 1.25s | 0.31s | 221,757,440 |
| 3-node, 100 Meta proposals | 0.10.0 | 0.126469 | 0.131350 | 0.165447 | 0.165447 | 0.165447 | 9.343s | 2.85s | 0.45s | 219,824,128 |

The 1,000-row and 25,000-row rows share one nextest process per tree, so their CPU and RSS figures are that process, not a separate figure for each size. p50 change on the persistent standalone path is +2.1% at 1,000 rows and +0.3% at 25,000 rows. The 3-node p50 change is +108%. That path uses `RaftManager::new`. On 0.9.25 that constructor had no persistent store, so `append_entries` completed the IO callback immediately. On 0.10.0 it uses `InMemoryBackend`, and a cluster group flushes the log before the append is acknowledged. The decision log records that measurement. The standalone RocksDB commit path stays under 5%.

## Idle Raft groups (T123)

Recorded 2026-11-17 on darwin 27.2.0, `dev` profile, `--retries 0`, `--offline`. Nextest run `db04e02d-6784-4cd3-921f-23ae45fa85ff`. Three nodes share one test process and use `RaftManager::new` (in-memory log). Heartbeat is 50 ms and the election timeout is (150, 300) ms. The idle window starts after `last_log_index` has been unchanged for 1 second, then lasts 3 seconds. RSS is `ps` resident memory of that test process (`rss` kilobytes times 1024). CPU is the `ps` user and system clock delta over those 3 seconds. Both are divided by the number of Raft instances in the process, so the per-instance figures include the test harness. `ps` clocks resolve to 0.01 second. Connections are outgoing channels in the shared per-node pool, summed across the three nodes at the end of the window. Heartbeat leaders and acks are the OpenRaft heartbeat maps. Heartbeat resets are `quorum_ack_age_ms` decreases seen on a 25 ms sample. Append writes are the change in the sum of `last_log_index` across instances. Snapshot throughput is local chunking and reassembly of one 4,194,305-byte payload, not a follower install.

```bash
cargo nextest run -p kalamdb-raft --offline --retries 0 --test t123_idle_groups
```

Default groups are 1 meta + 32 user + 4 shared = 37 per node (111 instances), RPC ports 17200–17202. The test took 107.300s. The 100-group case is 1 meta + 95 user + 4 shared (300 instances), RPC ports 17400–17402. That test took 38.629s.

| Case | Instances | RSS bytes | RSS / instance | Idle user s | Idle sys s | CPU s / instance | Connections | Heartbeat leaders | Heartbeat acks | Heartbeat resets | Append index delta | Snapshot bytes | Snapshot s | Snapshot bytes/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| default, 37 groups | 111 | 85,721,088 | 772,262 | 2.360000 | 0.040000 | 0.021622 | 1 | 37 | 74 | 1144 | 0 | 4,194,305 | 0.001284 | 3265427908.113772 |
| 100 groups | 300 | 114,802,688 | 382,675 | 2.950000 | 0.030000 | 0.009933 | 1 | 100 | 156 | 0 | 0 | 4,194,305 | 0.000650 | 6453600998.281320 |

The 100-group heartbeat map had 100 leaders and 156 acks, and the 25 ms quorum-age sample saw 0 decreases. Append index delta is 0 in both idle windows. The connection count is channels, not channels times groups: the groups on a node share one pool.

## Performance acceptance (T089)

Recorded 2026-11-17 on darwin 27.2.0, `dev` profile, `--retries 0`, `--offline`. The 16-script checkpoint above is unchanged. These runs are the missing peak-memory, prepared-CRUD, and churn rows. RSS is `ps` resident memory of the test process. Tracked bytes are `MemoryLedger` high-water marks; release does not lower them. RocksDB counters come from `StorageBackend::stats`. Disk and WAL bytes are the size of the test workspace, with WAL counted from `*.log` files and names containing `wal`.

```bash
cargo nextest run -p kalamdb-core --offline --retries 0 --test transactions -E 'test(t089_large_dml_memory)'
cargo nextest run -p kalamdb-core --offline --retries 0 --test transactions -E 'test(t089_prepared_crud_1_8_32)'
cargo nextest run -p kalamdb-core --offline --retries 0 --test transactions -E 'test(t089_churn_300s)'
```

The large-DML case stages 25,000 user rows and commits them. The test passed in 1.085s. Peak tracked bytes while the rows were staged were 12,027,788. The commit raised the simultaneous peak to 17,241,682 because the encoded batch was charged while the staged payload and overlay were still held. After commit, staged held bytes were 0. There is no earlier peak-tracked measurement, so the 30% lower peak goal is not shown.

| Point | RSS bytes | Peak tracked bytes | Staged held | Encoded peak | Memtables | WAL | Disk |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Before stage | 124,796,928 |  |  |  |  |  |  |
| Staged | 176,898,048 | 12,027,788 | 5,213,894 | 0 |  |  |  |
| After commit | 279,216,128 | 17,241,682 | 0 | 5,213,894 | 6,832,064 | 13,255,977 | 13,488,643 |

Staged payload peak was 5,213,894 bytes and overlay peak was 6,813,894. Scan, target cache, and side-effect peaks were 0. Live SST bytes were 0 and pending compaction bytes were 0. Memtable entries were 50,329.

Prepared CRUD prepares `INSERT`, `SELECT`, `UPDATE`, and `DELETE` once per client, with `$1`/`$2` parameters, and materializes every result. Eight rounds per client. User and shared each start their own app. The test passed in 3.068s.

| Engine | Clients | Ops | Seconds | Ops/s | p50 s | p95 s | p99 s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| user | 1 | 32 | 0.061064 | 524.042857 | 0.001005 | 0.006133 | 0.019492 |
| shared | 1 | 32 | 0.033691 | 949.801506 | 0.000841 | 0.003557 | 0.003903 |
| user | 8 | 256 | 0.261062 | 980.608561 | 0.009846 | 0.014767 | 0.017301 |
| shared | 8 | 256 | 0.306375 | 835.576293 | 0.011666 | 0.016169 | 0.017102 |
| user | 32 | 1024 | 0.964143 | 1062.082656 | 0.035223 | 0.056422 | 0.063534 |
| shared | 32 | 1024 | 0.949065 | 1078.956705 | 0.033889 | 0.052525 | 0.056644 |

The churn series is one user table, 256 live keys, 8 clients, mixed select, update, and delete plus reinsert. The cluster test app does not start the server flush loop, so the test calls `flush_all_memtables` every 60 seconds. The test passed in 333.100s. Samples are 30-second windows. Tracked peak stayed 2,853 bytes. RSS moved from 140,836,864 at 30s to 150,863,872 at 301s.

| t s | Window ops | Ops/s | p50 s | p95 s | p99 s | RSS | Memtables | Live SST | WAL | Disk | Pending compaction |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 30 | 36448 | 1203.551766 | 0.006928 | 0.013726 | 0.017358 | 140,836,864 | 295,872 | 247,121 | 444,116 | 3,140,423 | 0 |
| 61 | 35226 | 1164.262884 | 0.007135 | 0.014240 | 0.018148 | 142,704,640 | 12,288 | 436,388 | 0 | 5,111,797 | 0 |
| 91 | 35220 | 1163.711318 | 0.007174 | 0.014245 | 0.018412 | 143,949,824 | 156,608 | 610,209 | 206,206 | 7,533,753 | 0 |
| 120 | 33856 | 1156.269288 | 0.007206 | 0.014285 | 0.018763 | 144,736,256 | 12,288 | 716,666 | 0 | 9,507,580 | 0 |
| 150 | 34183 | 1130.395641 | 0.007242 | 0.014855 | 0.020937 | 145,899,520 | 385,840 | 803,139 | 946,492 | 12,705,439 | 0 |
| 181 | 32025 | 1053.644310 | 0.007642 | 0.016674 | 0.023040 | 147,701,760 | 12,288 | 994,300 | 0 | 13,911,582 | 0 |
| 210 | 30043 | 1025.780326 | 0.008034 | 0.016445 | 0.021168 | 147,849,216 | 279,464 | 1,179,030 | 736,759 | 16,639,539 | 0 |
| 240 | 32217 | 1066.614429 | 0.007748 | 0.015641 | 0.019616 | 148,373,504 | 12,288 | 1,200,696 | 0 | 18,013,804 | 0 |
| 271 | 32094 | 1060.747783 | 0.007784 | 0.015748 | 0.019743 | 149,749,760 | 344,256 | 1,330,581 | 835,802 | 20,969,074 | 0 |
| 301 | 31926 | 1053.803466 | 0.007854 | 0.015908 | 0.019978 | 150,863,872 | 12,288 | 1,601,407 | 0 | 22,441,279 | 69,406 |

A separate transaction then held a snapshot for 30 seconds while the workers continued. After `ROLLBACK` and another memtable flush, the process waited 2 seconds.

| Phase | Ops in window | p50 s | p95 s | p99 s | RSS | Staged held | Live SST | WAL | Disk | Pending compaction |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Pinned 30s | 31552 | 0.007841 | 0.015850 | 0.020416 | 151,224,320 | 946 | 1,711,091 | 317,064 | 24,859,071 | 0 |
| After unpin |  |  |  |  | 151,240,704 | 595 | 1,628,551 | 211,681 | 24,816,223 | 0 |

RSS did not fall after unpin. Staged held bytes fell from 946 to 595. Live SST fell from 1,711,091 to 1,628,551 and disk fell from 24,859,071 to 24,816,223. The remaining measured work is script 12 and the combined #8/#11/#12/#13/#15 goal, which is still 3.5% rather than 30%.

## T120 engine merge, 2026-11-17

The before row is the 2026-11-15 T088 and T089 checkpoint on `90fa854bdc01`. The after row is the uncommitted engine merge on `782d1738c` (`782d1738c11b`). The run directory name uses that commit. The merge itself is not in the commit. `KALAMDB_DML_STAGE_TRACE` was unset. Dev profile, one discarded warm-up suite and five recorded suites. The 5% gate is not met: scripts 1, 8, 9, and 16 are slower than the November 15 median by more than 5%. The speed follow-up is deferred.

### T088 after

Dev profile, Apple M5 Pro. Log `/tmp/kalamdb-t120-t088.log`. Started 2026-11-17T14:33:00Z, finished 2026-11-17T14:50:24Z, exit 0. This does not include a new CLI e2e run.

| Suite | Passed | Failed | Skipped | Summary | real | user | sys |
| --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |
| Gate crates | 843 | 0 | 0 | 2.557s | 494.11s | 939.04s | 46.46s |
| Core filter | 112 | 0 | 360 | 16.884s | 549.76s | 628.57s | 39.85s |

The core filter has one more passing test than the 111-test before row. The extra name is `kalamdb-core::transactions transaction_user_batch_commit::t112_standalone_insert_commit_times`, which matches the `transaction` filter.

### 16-script after

Run `benchv2/pgwire-speed/results/runs/20261117T145734Z-dev-782d1738c11b`. PostgreSQL total median 1.115s, range 1.102..1.131. KalamDB seconds:

| Script | November 15 median | After median | After range | Change |
| --- | ---: | ---: | --- | --- |
| 1 | 0.133 | 0.257 | 0.250..0.286 | +93.2% |
| 2 | 2.493 | 2.580 | 2.568..2.605 | +3.5% |
| 3 | 2.566 | 2.661 | 2.649..2.670 | +3.7% |
| 4 | 0.459 | 0.447 | 0.420..0.524 | −2.6% |
| 5 | 0.647 | 0.676 | 0.648..0.688 | +4.5% |
| 6 | 0.340 | 0.357 | 0.353..0.359 | +5.0% |
| 7 | 0.616 | 0.641 | 0.636..0.671 | +4.1% |
| 8 | 3.744 | 4.184 | 4.171..4.210 | +11.8% |
| 9 | 2.832 | 2.978 | 2.951..2.997 | +5.2% |
| 10 | 2.992 | 3.130 | 3.097..3.209 | +4.6% |
| 11 | 2.952 | 3.067 | 3.050..3.081 | +3.9% |
| 12 | 0.674 | 0.696 | 0.689..0.723 | +3.3% |
| 13 | 1.910 | 1.992 | 1.967..1.999 | +4.3% |
| 14 | 1.508 | 1.537 | 1.532..1.549 | +1.9% |
| 15 | 2.531 | 2.641 | 2.606..2.659 | +4.3% |
| 16 | 0.062 | 0.073 | 0.067..0.090 | +17.7% |
| total | 26.458 | 27.927 | 27.893..27.997 | +5.6% |

Scripts 1, 8, and 9 have ranges that do not overlap the November 15 ranges. Script 16 is 0.011s slower and its range overlaps 0.061..0.070. Script 6 is 0.017s slower, which is 5% of 0.340s and not more than 5%. Combined scripts 8, 11, 12, 13, and 15 are 4.184 + 3.067 + 0.696 + 1.992 + 2.641 = 12.580s, against 11.811s on November 15.

A separate traced sample of script 1, not part of the median, spent 20ms in `backend_submit` (1,028 hits) and 54ms in `scan_filter` (70 hits) inside a 0.275s wall time. The storage submit is not the doubled second. The benchmark's autocommit shared inserts still take the existing `propose_shared_insert` path when Raft is started. That proposal cost was not separated from the engine merge.
