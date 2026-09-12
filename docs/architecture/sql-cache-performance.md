# SQL cache performance

## Prepared metadata ownership

`PreparedExecutionStatement` shares its immutable `SqlStatement` classification
and sqlparser `Statement` through `Arc`. Moka clones values on cache lookup;
previously that recursively cloned the DML expression tree and classification
on every hit. PostgreSQL wire statement/portal clones use the same metadata.
Sharing these trees avoids allocations proportional to the parsed statement's
size and allows concurrent requests to retain one parsed tree.

The constructor continues to accept owned parsed values. Callers that need a
borrowed AST use `parsed_dml.as_deref()`. Parameter binding and volatile default
evaluation remain per execution. Shared metadata contains no bound parameters,
execution state, user session, or query results. Cache invalidation removes
lookup visibility while existing readers retain their references safely.

The SQL text and table identifier still clone. This change reduces allocation
work and duplicate live AST memory; it is not a byte budget for all SQL caches.

## Review of proposed engine optimizations

Checked against the working tree on 2026-09-11:

PostgreSQL's useful comparison is reusable generic plans versus parameter-specific
custom plans, rather than simply a cached bound portal. PostgreSQL can choose
either and can replan after schema or statistics changes; see its
[PREPARE documentation](https://www.postgresql.org/docs/18/sql-prepare.html).

| Proposal | Current behavior and remaining work |
| --- | --- |
| Physical plan caching | The general cache contains optimized logical plans. General execution still binds and physically plans. Cached supported point gets already call the provider directly. Physical plan reuse needs execution-local parameters, identity, snapshot, and fresh scan state; a shared execution object is unsafe without that separation. |
| Direct autocommit | Supported literal and parameterized VALUES inserts already run through `try_execute_literal_insert_via_applier` before the DataFusion fallback. The fallback for user/shared DML still opens and commits a request transaction. Removing it requires preserving all-or-nothing statement execution, including multi-row updates and inserts from queries. |
| Raft grouping | Each proposal calls `client_write`, but this does not establish one network round per concurrent write. The log store accepts multiple entries and calls `append_logs` on the batch. State-machine application still loops through entries and records progress per entry. Profile apply and persistence before introducing another batching queue; preserve per-statement errors, ordering, and bounded queue memory. |
| Hot-only storage | Writes consult the manifest, and cold PK checks already return early for an empty segment list. No `FLUSH_POLICY` is not proof that cold data cannot exist: manual `STORAGE FLUSH TABLE` remains available. Skipping cold reads needs a durable storage invariant and transitions that handle existing segments. |
| Default ordering | `optimized_plan_for_cache` injects default ordering before optimizing. This is a real potential sort cost when eligible. Removing it changes result ordering; first establish the intended ordering contract and benchmark scans separately from point gets. |
| VALUES compiler | `FastInsertMetadata` is already used by ordinary supported VALUES inserts, including bound parameters, through `try_build_literal_insert_rows`. It is not limited to explicit transaction batches. Unsupported shapes fall back to DataFusion. |

## Further measurements

Measure cache hit rate, admission/eviction, retained bytes, allocation counts,
planning time, provider scan time, manifest work, and Raft apply/persistence
separately. The plan and prepared metadata caches are bounded by entry count
and idle lifetime, not retained bytes. Any byte accounting should include keys,
ASTs, expressions, schemas, and provider references while documenting shared
allocation estimates. Keep diagnostic traversals off the hit path.

Use the pgwire comparison for PostgreSQL comparisons, with matched durability,
hardware, concurrency, data, and consumed results. Record source revisions and
repeat runs. A cache microbenchmark improvement alone does not establish an
end-to-end database speedup or a PostgreSQL win.

Run the isolated cache benchmark in the default test profile:

```sh
cargo nextest run -p kalamdb-core --test prepared_metadata_cache \
  --run-ignored only --nocapture
```

It measures 20,000 warm Moka hits for one-row and 128-row INSERT metadata,
three times each, with parsing outside the timed region. Timings are printed
in seconds and have no CI threshold.

### Measured result (2026-09-11)

Same arm64 host, default unoptimized test profile, Rust
`1.100.0-nightly (67854e511 2026-08-15)`, working tree based on `2d1aa7f8`.
Median of three samples; each sample performs 20,000 hits:

| Cached INSERT metadata | Before (seconds) | Shared trees (seconds) | Lookup speedup |
| --- | ---: | ---: | ---: |
| One VALUES row | 0.038189 | 0.008267 | 4.62x |
| 128 VALUES rows | 0.777096 | 0.008659 | 89.74x |

Before samples: one row `0.038295, 0.038189, 0.038178`; 128 rows
`0.778277, 0.777096, 0.773997` seconds. After samples: one row
`0.008278, 0.008222, 0.008267`; 128 rows
`0.008604, 0.008670, 0.008659` seconds.

The benchmark uses an integer key to isolate cached-value cloning; it does
not measure SQL cache-key allocation/hashing, network latency, planning,
execution, or database throughput. The sharing tests verify that cache hits
reference the same AST/classification and that retained AST readers survive
cache invalidation. Process RSS and total retained cache bytes were not
measured.

## HTTP cached PK point-get (skip Arrow)

The comparison bake-off is one HTTP SQL request per row:

```sql
SELECT id, owner, room, data FROM bench.message WHERE id = $1
```

After the logical plan is cached, `SqlExecutor::try_execute_cached_point_get`
calls the table provider directly. The HTTP JSON path must keep
`ExecutionResult::ScalarRows` (`Row` maps → `rows_to_json_arrays`). Converting
to a RecordBatch first (`json_rows_to_arrow_batch` then
`record_batch_to_json_arrays`) is a measured regression.

Same arm64 host, release `kalamdb-server`, comparison harness, 1M point reads,
concurrency 16, 2026-09-11:

| HTTP point-get encoding | 1M reads | read p50 |
| --- | ---: | ---: |
| Row → RecordBatch → JSON (tonight inline baseline) | 16.88s | 245µs |
| Row → JSON (`ScalarRows`) | 15.30s | 220µs |

Do not "simplify" the scan by returning `None` from `produce_scalar_rows`
whenever `physical_filter` or `output_projection` is set. DataFusion marks
`id = $1` as Exact, so those residuals are always present on this query. The
hot PK lookup already applied the equality. Residual Arrow filtering is only
required when the **full** scan filter list is more than a single PK equality
(AND predicates, non-PK filters). Use `authorization_filter` for that check,
not the inexact/source-pruning subset (`filter`), which is PK-only even for
`id = $1 AND name = $2`.

Aliased projections (`SELECT name AS title`) keep scan-row keys as catalog
names. The cached point-get path remaps with `project_point_get_scalar_rows`
using the logical Projection exprs. `align_rows_to_output_schema` only remaps
residual scan projections (Exact `id = $1` can leave extra columns such as
`id` on the scan output). Looking up the alias in the unmapped `Row` yields
JSON null. Unmapped aliases must return `None` and fall back to Arrow — never
emit a title column of null.

Protecting tests:

- `cached_pk_point_get_skips_arrow_after_plan_cache_hit` in
  `backend/crates/kalamdb-core/tests/sql_cached_point_get.rs` — cached
  `SELECT id, name WHERE id = $1` and `SELECT name WHERE id = $1` must return
  `ExecutionResult::ScalarRows`; `id = $1 AND name = $2` stays on Arrow.
- `cached_point_get_projects_non_pk_columns_across_table_types` in the same
  file — cached `SELECT name AS title WHERE id = $1` must stay on ScalarRows
  and keep the output name `title`.
- `project_point_get_scalar_rows_remaps_alias_when_scan_keeps_pk` in
  `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs` —
  projection exprs remap `name` → `title` when the scan still has `id`.
- `simple_pk_equality_allows_skip_arrow` and
  `align_rows_to_output_schema_applies_alias_via_output_projection` in
  `backend/crates/kalamdb-tables/src/utils/base.rs` — AND conjunctions are
  not skip-Arrow-eligible; residual scan projections remap through
  `output_projection`.

Related measured non-wins on the same bake-off (do not re-try without new
evidence):

- `spawn_blocking` around the PK index lookup: 17.83s / p50 253µs. Keep the
  reverse `limit=1` lookup inline (`get_latest_by_index_prefix_async`).
- Selected-column KOBJ decode when the projection lists every stored user
  column: 17.31s / p50 252µs. `storage_ordinals_for_scan` returns `None` unless
  the projection is a strict subset of live stored fields.

The 6 Sep 22:27 comparison (12.48s / p50 174µs) is not explained by either of
those two changes. Skip-Arrow recovered ~1.6s / ~25µs of that gap.
