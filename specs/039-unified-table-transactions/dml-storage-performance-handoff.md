# DML, query, storage, and transaction performance implementation handoff

Status: implementation proposal, not a claim of measured improvements.
Prepared: 2026-10-30. Scope: current KalamDB working tree.

## Objective and boundaries

Reduce ordinary INSERT, UPDATE, DELETE, SELECT, and transaction overhead relative to PostgreSQL using `benchv2/pgwire-speed`, while reducing allocation, peak memory, and storage write amplification. Optimize shared execution/storage foundations rather than special-casing benchmark SQL.

Preserve existing uncommitted work. The reviewed tree is actively changing, including transaction folding and target-resolution work. Revalidate each finding against current code before editing; extend or verify an existing implementation instead of duplicating it. Do not reset files, commit, or introduce an unrelated architectural rewrite.

Implement the phases below incrementally. Phase E contains conditional investigations, not mandatory storage-format migrations. Do not implement every possible optimization simply because it appears here. Keep a compact evidence ledger: finding, change, measured outcome, remaining issue.

Follow repository AGENTS.md: one model per file; typed IDs/scopes; `Arc<AppContext>` for orchestration; common user/shared processing; DataFusion for SQL semantics, visibility and filtering; SQL parsed once through KalamParser. RocksDB belongs in `kalamdb-store`; file/Parquet lifecycle belongs in `kalamdb-filestore`.

## Architecture acceptance: shared, simple, and maintainable

Architecture quality is a completion gate alongside correctness and measured performance. A faster result does not pass if it introduces duplicated execution pipelines or unexplained special cases. This plan cannot guarantee future code quality; the implementing agent must demonstrate these conditions in the resulting diff.

### Ownership and shared processing

| Responsibility | Owner and boundary |
|---|---|
| SQL parsing, expressions, filtering, query optimization | Existing KalamParser/DataFusion integration; no parallel parser, expression engine, or hand-coded SQL matcher |
| DML preparation and table behavior | Shared table processing in `kalamdb-tables`; USER/SHARED adapters supply scope, keys and storage-specific operations |
| Staging, folding, overlays, claims and transaction memory | Existing `kalamdb-transactions` abstractions; core coordinates the lifecycle rather than duplicating these algorithms |
| Index encoding/maintenance and atomic KV submission | Existing index/store abstractions in `kalamdb-store`; providers supply schema and table policy |
| Replication, apply ordering and persistence boundary | Existing Raft/applier interfaces; no separate commit protocol for a fast path |
| Parquet/files, cleanup and compaction | `kalamdb-filestore`; core orchestrates without embedding storage implementation details |

USER and SHARED must share mutation preparation, batching, validation and effect scheduling wherever semantics agree. Keep differences in small, typed adapters at the boundary, rather than copying a whole method for each table type. Share applicable primitives with STREAM tables too, while preserving stream expiry, deletion and transaction restrictions; do not manufacture a common transaction model that streams do not support. No new crate is required by this plan.

### Rules for every optimization

- Extend an existing abstraction before introducing another cache, executor, planner, batch type or memory tracker. A new abstraction must own a real invariant or hide implementation complexity, not merely forward calls. Avoid generic frameworks, callback chains, giant traits and combinations of boolean mode flags.
- Use one canonical mutation representation and one shared lifecycle. Single-row operations should use the same policy as batches. If a measured scalar specialization is necessary, keep it local beneath the shared interface; do not clone authorization, constraints, MVCC or side-effect logic.
- A fast path may specialize a physical operation with explicit applicability rules. It must reuse the shared semantic checks and general fallback. Never branch on benchmark IDs, table names, particular constants, client identity or SQL text patterns. Unsupported operations may fall back; corruption, permission and execution errors must remain errors.
- Consolidate while replacing: remove superseded production paths, wrappers, aliases and temporary feature switches in the same completed slice. A shared facade over two duplicate implementations does not count as consolidation. Keep any required compatibility path narrow, documented and tested with a concrete removal condition.
- Keep ownership and lifetimes visible in function signatures. Use typed states where they prevent invalid transitions, not a proliferation of near-identical row/commit models. Reuse immutable schemas and buffers without adding permanent caches to hide repeated work.
- Avoid unrelated cleanup. Measure production-code additions/deletions in the touched consolidation scope, excluding tests, docs, generated output and formatting. Consolidation tasks must reduce duplicated policy and yield a net production-code reduction in that scope; do not game this by compressing readable code. New capabilities may add code, but must justify the added surface and replace any equivalent old mechanism.

### Review gate for each completed slice

Before implementation, record a short ownership/call-path map for the touched code and identify the old paths to retire. After implementation, report the shared entry point, remaining table-specific differences, deleted paths, production-code delta, and any added public interfaces/configuration. Run common behavior cases through USER and SHARED adapters using shared test fixtures where practical, plus focused tests for actual semantic differences. Review the diff for duplicated validation, dispatch, error handling and effect ordering. If the speedup requires architectural debt, simplify the design or leave that optimization unshipped and explain the tradeoff.

## Baseline and prioritization

The user-provided saved October 27 run totals 6.460s for KalamDB versus 1.168s PostgreSQL. It predates current edits and cannot establish current performance.

| Workload | Saved KalamDB seconds | Saved PG seconds | Design target |
|---|---:|---:|---|
| #4 unindexed numeric SELECTs | 0.650 | 0.079 | Projected scan and aggregation |
| #5 string SELECTs | 0.850 | 0.195 | Efficient scan and string evaluation |
| #8 unindexed UPDATEs | 0.613 | 0.030 | Scan, overlay, assignment, mutation preparation |
| #11 INSERT SELECT | 0.405 | 0.037 | Columnar pipeline and staging ownership |
| #12 substring DELETE | 0.181 | 0.005 | Scan plus bulk mutation |
| #13 range DELETE | 0.732 | 0.006 | Access choice, target reads, index maintenance |
| #15 DELETE plus small INSERTs | 0.472 | 0.034 | Transaction memory and commit preparation |

#13 is one `DELETE FROM t2 WHERE a>10 AND a<20000`, not thousands of client round trips. #8 has 1,000 range updates in a transaction. #4/#5 are aggregate scans. Large indexed inserts/updates are already relatively closer; protect them from regression. Prioritize absolute seconds saved as well as ratios.

## Source findings and existing capabilities

All paths below are repository-relative. Symbols are better anchors than line numbers in this moving tree.

| Area | Current evidence | Implication |
|---|---|---|
| Delete staging | `kalamdb-tables/src/versioned/engine/user/sql.rs::delete_from` collects rows, extracts PKs and may recheck liveness after claiming a key | Reuse observations only with correct concurrency validation; measure actual repeated reads |
| Commit | `kalamdb-core/src/applier/executor/dml.rs::prepare_mixed_versioned_commit`, `submit_prepared_commit` | One atomic operation batch already exists; preparation and retained payloads remain opportunities |
| Target cache | `kalamdb-tables/src/versioned/target.rs::TargetResolutionCache` is newly present | Verify it is populated by authoritative loads, not merely already-written rows; an insertion counter alone does not prove one storage read |
| Folding | `kalamdb-transactions/src/write_set.rs::fold_commit`, coordinator `commit` | Folding already exists; borrowing the write set can retain original history/overlay alongside folded mutations |
| Overlay | `TransactionOverlay` uses Arc-backed maps; `TransactionOverlayExec::merge_partitions_with_overlay` materializes row maps | Cheap snapshot cloning already exists; columnar execution and copy-on-write amplification still need measurement |
| DML | `utils/datafusion_dml.rs::collect_matching_rows_with_projection` collects Arrow then converts to rows; UPDATE already uses `PreparedUpdateAssignments` | Avoid redundant conversion, not another expression evaluator |
| Index reads | `indexed_store.rs::scan_by_index_plan_iter_selected` fetches up to 32 keys with `get_many`; RocksDB overrides it with `multi_get_cf` | Multi-get already exists; measure batch sizing and key/decoded-buffer copying |
| Access selection | `plan_index_scan_from_conjuncts` chooses an index by `IndexScanPlan::score` | No row-count-based comparison against a sequential scan at this decision point |
| Index writes | `index/prefix.rs::extract_key` includes row sequence/version; `IndexedEntityStore::push_replace` maintains indexes on replacement | Unchanged logical values can still cause index rewrites |
| Coalescing | `write_coalesce.rs::buffer_put`/`buffer_delete` construct owned operations before detecting an active task-local session | Avoidable key/value copies on the inactive path; global buffer/hook mutexes also merit contention measurement |
| Memory limits | `StagedMutation::approximate_size_bytes` primarily estimates mutation payload; write sets also retain overlays, maps, history, fold output and prepared operations | Current transaction byte accounting is not a complete peak-memory budget |
| RocksDB memory | Shared block cache and WriteBufferManager already exist; `db_write_buffer_budget` includes a CF-count-dependent arena floor | Do not propose adding a shared cache; measure total resource growth with table/index count |

Crate paths in this table are under `backend/crates/`.

## Non-negotiable correctness gates

1. Preserve SQL-visible results, affected-row counts, read-your-writes, savepoints, NULL/type semantics, constraint checks, and current isolation/error behavior. Do not silently turn SQL errors into fallbacks or skipped rows.
2. Keep atomicity across all supported tables/index partitions in one commit group, plus the applicable replay/frontier metadata. Reject unsupported cross-group writes before mutation. Bounded processing chunks must NOT become separately visible commits.
3. Do not reuse a scan row as an authoritative current row without validating its version under the existing serialization/conflict mechanism. If a writer waited on a claim, recheck the applicable current predicate/version according to the isolation contract.
4. Verify actual snapshot-bound propagation. In the reviewed code, a `check_expected_max_version(version, None)` call exists; that call does not prove a snapshot conflict check. Determine where validation really occurs. Preserve replicated apply determinism; a leader-only cache must not change follower/replay behavior.
5. Preserve old versions required by pinned snapshots and branch/history contracts. Compare domain-compatible encoded versions, not raw Raft indexes against encoded row versions. Resolve unknown/unpinned snapshot semantics conservatively. No timer-based reclamation.
6. Do not replace DELETE with TRUNCATE or a raw range deletion. A range predicate on a secondary column is not a safe physical key-range deletion, and tombstones may hide cold data.
7. Publish notifications, visibility frontiers, and post-persist effects at their existing successful persistence boundary. Coalescing makes an inner `batch()` return insufficient evidence of persistence. Never acknowledge early or silently lose live changes.
8. Keep authorization/user scope in scans, caches, indexes and claims. Preserve file cleanup and vector-index work when applicable.

## Phase A — establish evidence and remove small avoidable costs

### A1. Reproducible benchmark and attribution

Files: `benchv2/pgwire-speed/{src/main.rs,run.sh,README.md}`, existing observability hooks and touched DML/storage paths only.

- Preserve saved results. Store new runs in distinct directories with source commit, dirty-diff fingerprint, binary fingerprint, toolchain/profile/features, SQL file hashes, server settings and timestamps.
- Adapt the runner to respect the default-dev-profile rule: the existing script hardcodes release. Establish a new same-profile before/after baseline. Do not compare dev results to October release timings as a regression. Do not build release without an explicit instruction overriding the repository rule.
- Use one discarded warm-up suite and at least five recorded complete suites per before/after version. Reset data for each suite; preserve the 16-script order/dependencies. Alternate engine run order across repetitions; document cache conditions. Do not time one dependent script against an arbitrary database state.
- Record median and range per script and total, in seconds. Record exact settings: one connection, one simple-query message per script, WAL enabled, fsync-wait policy matched, hot-only flush configuration. Keep server startup/auth outside SQL timings.
- Add low-overhead aggregate counters/timing spans for planning, scan/filter, overlay, staging/folding, target resolution, row encoding/index construction, backend submission, and post-commit work. Do not emit per-row logs or high-cardinality PK labels. Distinguish nested/overlapping timings; do not attribute unexplained residual time by subtraction.
- Count actual backend lookups, decoded rows/bytes, index entries visited, operations/WAL bytes, and target-resolution misses. A cache's bookkeeping counter is not a replacement for backend instrumentation.
- Validate aggregate values and final table contents outside timing. Existing row/count checks miss wrong updated values and averages. Use deterministic ordered checksums or canonical multiset comparison, preserving NULL, types and duplicate multiplicity; use appropriate float tolerance.

Acceptance: a reproducible current baseline and a short table showing which stages dominate #8, #11, #12, #13 and #15. Instrumented profiling runs must be distinguished from timing runs if overhead is material.

### A2. Avoid inactive coalescing allocations

Files: `kalamdb-store/src/write_coalesce.rs` and focused store tests.

Check for the active coalescing context before cloning a partition/key/value. Allocate owned operations only when buffering is actually possible. Preserve direct writes, active buffering, cancellation cleanup, and deferred-hook semantics. Preserve or explicitly reject nested coalescing according to the existing contract.

Acceptance: inactive helper path performs no key/value-buffer allocation; existing behavior tests pass; report allocation counts and runtime for direct and coalesced paths. Do not redesign the whole coalescer for this task.

## Phase B — transaction ownership and bulk mutation preparation

### B1. Consume the write set when committing

Files: transaction `write_set.rs`, `staged_mutation.rs`, coordinator `commit`, focused transaction tests.

- Introduce an ownership-consuming fold/seal path after transaction state becomes non-writable. Move payloads rather than cloning them wherever possible.
- Release overlay maps, latest-key maps and obsolete history before awaiting Raft/backend commit. Preserve everything needed to validate even an empty/net-no-op folded commit; do not bypass group admission because folding produced zero writes.
- Keep ordered history necessary for active savepoints. Folding final stored state must not change statement counts, required events, constraints or file cleanup. Do not eagerly erase history solely to meet a memory target.
- Share immutable payloads or batch slices where justified; avoid putting Arc around every scalar. Bound retained parent batches so one small slice does not pin a huge allocation.

Acceptance: tests cover insert-update-delete, delete-reinsert, repeated update, PK move, duplicate inserts, savepoint rollback/release and multi-table same-group commits. Show lower peak retained bytes for 25k distinct keys and 25k mutations of one key. At the commit await, the original write-set/overlay payload must not remain accidentally owned.

### B2. Resolve each distinct mutation target once per authoritative preparation

Files: applier `dml.rs`, existing target cache, provider operations, user/shared mutation storage paths.

- Reuse `ScopedTargetKey` and the existing target cache; do not create another string-keyed cache. Use a distinct type for authoritative baseline versus transaction-local final state if needed.
- Resolve targets in bounded batches grouped by table/scope. Retain row/version/index-relevant fields only as long as needed. Handle missing rows and tombstones explicitly. Preserve cold-row fallback and schema-version decoding.
- Separate safety revalidation from full payload loading. A second full decode should not be required just to check a version. Prove where claims/commit ordering make reuse safe, including concurrent writers.
- Prepare row and secondary-index operations together; reuse the same prior row for index extraction, files and old-value notifications. Avoid retaining both complete before/after rows when the enabled effects do not need both.
- Compare isolated range DELETE with the current path before extending the same mechanism to UPDATE. Implement shared policy once and retain table-specific storage adapters.
- Complete the slice through the common USER/SHARED implementation and remove replaced preparation paths. Do not leave the first optimized table type on a separate permanent executor.

Acceptance: actual counters show at most one full authoritative baseline decode per distinct target during preparation, excluding explicitly documented conflict retries. Separate scan/projection reads from this count. Selective and broad deletes preserve exact contents, snapshots, counts and events; batch failure yields no partial visibility.

### B3. Batch post-commit work by table and scope

Files: `prepared_commit.rs`, provider finish methods, side-effect plan.

Collapse repeated manifest/pending-write marks. Build vector work only for relevant indexes and notification payloads only for actual consumers/required durable delivery. Keep per-row semantic events when required; batching dispatch is not permission to drop them. Release preparation buffers before effect dispatch when no longer needed.

Acceptance: unwatched, non-vector workloads do O(touched table scopes) bookkeeping rather than repeated identical per-row bookkeeping. Watched/vector/file cases preserve all required work. Failure injection proves no effects escape an unpersisted coalesced commit and no retry duplicates committed mutations.

### B4. Make claims scale with the transaction's own targets

Files: `kalamdb-transactions/src/pk_claims.rs` and its callers.

The reviewed claim map uses a single mutex and `release_owner` traverses all claims. Measure before changing synchronization. If contention or release time is material, use scoped typed keys and an owner-to-owned-keys index, with consistent acquisition/release ordering. Equal PKs in different user scopes must not serialize unnecessarily. Handle overlapping multi-key acquisition without indefinite deadlock; preserve the current conflict/isolation contract.

Acceptance: 1/8/32-client disjoint and overlapping-key tests, cancellation/abort cleanup, no missed wakeups, no retained owner entries after transactions finish. Releasing one owner should scale with that owner's claims, not every active claim. Do not expect this change alone to improve a single-connection suite substantially.

## Phase C — bounded columnar query and DML execution

### C1. Keep Arrow batches through matching and assignment evaluation

Files: `kalamdb-datafusion-sources/src/dml.rs`, table `utils/datafusion_dml.rs`, provider SQL methods.

Replace eager `collect -> Vec<Row> -> Arrow -> Vec<Row>` transitions with bounded RecordBatch streams for predicate evaluation, assignment evaluation and INSERT SELECT input. Reuse `PreparedUpdateAssignments` and existing serialization batch APIs. Project only needed columns plus PK/MVCC metadata; decode payload columns late. Keep DataFusion as the expression/visibility engine.

Convert only final mutations to the storage representation. Budget staging growth separately from scan workspace: streaming the input does not make a transaction's write set constant-memory. Do not stage mutations while a scan can observe those same new mutations and update a row twice (Halloween problem). Retain a statement-consistent input view or a budgeted/spillable target set as required.

Acceptance: #8/#11/#12 preserve semantics; wide unused text/binary columns are not decoded; input/assignment scratch allocation is bounded by configured batch-byte limits, not total matched rows. Confirm a large single row cannot bypass a row-count-only bound.

### C2. Stream transaction overlays

Files: transaction `overlay_exec.rs`, `overlay.rs`, table base provider.

- Preserve the existing Arc-backed overlay snapshot and empty-overlay bypass. Do not clone the full map per row or hold a growing map snapshot throughout staging by accident.
- Merge committed batches with the immutable statement overlay by scoped PK. Skip tombstones, patch matching base rows, append transaction-local inserts exactly once, and re-evaluate predicates whose truth may change.
- Handle updates moving a row into or out of a filter, including secondary-index predicates. A partial patch is not a complete row; fetch missing required base columns or use a safe fallback.
- Apply SQL LIMIT after visibility/overlay/filter semantics. Keep ORDER BY semantics in DataFusion. Use a budgeted seen-key structure only where required; do not re-materialize the entire committed table merely to track overlay keys.

Acceptance: committed-row working memory scales with batch size plus necessary overlay bookkeeping, not scanned-row count. Tests cover changed predicate columns, unchanged projected columns, delete+insert same PK, user isolation, NULLs, LIMIT, empty input, and repeated scans during one transaction.

### C3. Stream hot scans and preserve global MVCC resolution

Files: base hot/deferred scan code, versioned scan adapters, serialization batch decoder, store iterator APIs.

Decode projected hot records directly into Arrow using existing batch codecs where valid. Push filters only across operators where doing so cannot expose an older version. Do not resolve versions independently per arbitrary storage chunk: versions of one logical key may span batches. Keep global correctness through the existing DataFusion resolution or a proven ordered, budgeted plan. Use DataFusion memory reservations/spill facilities where applicable.

Replace hidden unbounded-materialization assumptions with iterator continuation or explicit resource-limit errors. The current unbounded scan guard is a correctness guard; do not simply remove or raise it. Exercise data beyond its bound and long version chains.

Acceptance: #4/#5 correctness and speed measured; scans beyond 100k raw rows are complete or fail explicitly, never truncate silently. Peak working memory is bounded or accounted/spilled. Old snapshots remain correct across chunk boundaries and hot/cold transitions.

## Phase D — index selection and storage memory

### D1. Compare index access with sequential access

Files: store index seek/planning, provider scan planning, existing statistics ownership.

Add inexpensive estimates of live row count, selectivity and fetched bytes. Compare sequential projected scan with index walk + row fetch + visibility resolution. Include a conservative path for missing/stale statistics. Do not scan the table to choose a plan on every request. Keep statistics scoped and bounded; use the existing DataFusion statistics surface when practical.

Expose chosen access and estimated/actual rows in diagnostic plans. Start with scalar equality/range predicates, including composite prefixes already supported. Do not introduce a second SQL optimizer. Benchmark selectivity approximately 0.01%, 1%, 10%, 50%, 90%, plus narrow/wide rows; tune from evidence rather than hardcoded benchmark IDs.

Acceptance: selective queries retain fast indexed access; broad range cases can select sequential access when measured cheaper. Both access paths return identical data under snapshots/overlays. Measure #7/#9/#10/#13; no unbounded statistics cache.

### D2. Reduce storage submission allocations without changing atomicity

Files: store `indexed_store.rs`, `storage_trait.rs`, RocksDB `backend.rs`, coalescing integration.

Start with allocation measurements around `Vec<Operation>`, prefixed physical keys, per-operation partition-name strings, native WriteBatch construction, and prepared effect payloads. Reuse resolved partition handles within a batch without allocating a partition-name String on every lookup. Reserve capacities from accounted estimates and reuse scratch buffers where lifetimes permit.

If the generic operation vector plus native batch is a measured peak-memory bottleneck, design an owned, backend-neutral atomic batch builder in `kalamdb-store`. Move encoded buffers into it and transfer memory reservations across ownership boundaries. Do not leak RocksDB objects into core/tables. Integrate with outer Raft coalescing so data, indexes and required metadata remain one atomic submission. No per-chunk commit workaround.

Acceptance: output operations and failure/replay behavior match the current batch path; measure allocations, peak bytes, WAL bytes and seconds. Default scope is allocation reduction; implement a new builder only if measurements justify the additional interface.

If introduced, the builder must integrate with or replace the existing batch mechanism behind one store interface. Do not retain competing commit APIs for scalar, bulk, user, shared and coalesced writes.

### D3. Account for transaction, query and commit memory separately

Files: mutation/write-set accounting, coordinator admission, prepared batch accounting, existing DataFusion/store memory controls.

Define an ownership ledger: staged payload/history, overlay/map overhead and capacities, scan/decoded batches, target cache, encoded operations/native batch, side-effect payloads, RocksDB memtables/cache. Distinguish RSS from tracked retained memory; allocator caches may retain RSS after logical release.

- Enforce per-transaction and process-wide admission before material expansion, not after constructing an oversized batch. Reuse existing configuration where it fits; document any new budget.
- Account conservatively for strings, nested Arrow arrays, hash-map capacity, concurrent transactions and temporary transition peaks. Shared Arc buffers should have one owner charge or a documented conservative charge, not disappear from accounting.
- Transfer/release reservations on commit, rollback, cancellation, errors and savepoint rollback. No global budget lock or DashMap guard across await.
- For oversized atomic transactions, fail with the existing compatible resource error or implement an explicitly designed staging/spill strategy. Do not silently split commits or claim constant memory while retaining all final operations.

Acceptance: adversarial wide rows, nested values, many small rows and concurrent transactions trigger predictable bounds. Logical live bytes return to baseline after cleanup, allowing documented allocator/RocksDB retention. Sum of admitted work remains bounded at process level.

## Phase E — evidence-gated storage/index design investigations

These are follow-ups after A–D measurements, not prerequisites to ship smaller wins.

### E1. Stable secondary-index references

Measure per-update secondary-index deletes/puts when indexed values do and do not change. If unnecessary index churn remains dominant, write an ADR comparing current version-addressed entries with stable logical-key entries and any current-version indirection.

The design must cover old snapshots, indexed-value changes, historical membership, uniqueness, covering queries, stale entries, cold segments, flush/compaction, migration and mixed-version recovery. Stable references trade fewer index writes for possible extra reads. Do not simply skip maintenance while index entries still point to obsolete physical row versions. Require correctness tests and a measured read/write/memory tradeoff before a format migration.

### E2. Compact tombstones

Deletes currently retain row fields in tombstone entities. Measure encoded bytes and retention duration. Explore storing only PK/version/deletion metadata when old values live elsewhere and notifications, recovery, indexes, files and historical queries do not require the deleted payload there. Preserve schema-version compatibility and masking of cold rows. This needs a format/retention design before implementation; it is not just clearing `fields`.

### E3. CF growth, compaction and snapshot-safe reclamation

Measure 10/100/1,000 table/index configurations: CF count, memtable floor, cache use, SST count/size, pending compaction bytes, stalls and file descriptors. The existing shared manager does not eliminate per-CF overhead; its current budget floor grows with family count.

Determine whether overhead warrants bounded CF grouping or explicit admission/resource policy. Do not switch existing partition layouts or merge CFs without compatibility and migration design. Shared CFs trade memory/file overhead against compaction/noisy-neighbor isolation.

Locate current reclamation ownership instead of assuming older hot-GC modules still exist. Test bounded maintenance with a pinned reader, then release the reader and verify reclamation catches up. Preserve tombstones needed by cold segments and unknown manifest state. A fixed timer is not a snapshot proof. Do not tune compaction/cache sizes to hide growing logical history.

## Additional improvements — planning, cold reads, and result delivery

These extend the shared pipeline rather than adding specialized executors. Source checked on 2026-10-30; runtime gains are unmeasured. Select them by workload evidence rather than assuming all improve the single-connection, hot-only speed suite.

### F1. Coalesce concurrent builds of the same logical plan

Evidence: `kalamdb-plan-cache/src/lib.rs` already exposes `try_lead`, `finish_lead`, and `cancel_lead`, but the inspected SELECT/DML executor paths perform ordinary cache get/plan/insert without using these methods. Simultaneous misses can allocate and optimize equivalent plans independently.

Extend the existing cache with bounded, cancellation-safe coordination for one plan build per complete cache identity. Awaiters reuse the completed immutable template, then bind their own parameters and execution context. Keep planning outside locks. Add proper asynchronous completion/wakeup handling; polling `try_lead` in a loop is not a design. Ensure leader cancellation, build errors, non-cacheable plans, admission rejection and epoch changes release waiters and in-flight entries. Share a transient successful result with current waiters even when retention admission rejects it, subject to valid identity/epochs and memory accounting. Never share session-dependent catalog-function results or captured transaction/user state across requests.

Acceptance: synchronize 32 requests for one uncached eligible query and observe one actual planning attempt; all receive correct independently bound results. Different keys still plan concurrently. Test cancellation, DDL/permission invalidation and oversized/non-cacheable plans without leaks or hangs. Report peak planning bytes, planning CPU and p95 latency against the independent-build baseline. This primarily benefits concurrent cold-cache traffic, not already-warm single-connection execution.

### F2. Remove repeated concatenation from cold Parquet scans

Evidence: `kalamdb-tables/src/manifest/planner.rs::append_record_batch` concatenates `[existing, next]`; `scan_parquet_files_async` invokes it for every incoming batch. For equal-sized materialized arrays, cumulative copying can grow as 1+2+...+N batches, while old and new aggregate buffers temporarily coexist.

First eliminate repeated whole-prefix copying. If the current return contract requires one batch, use budgeted accumulation plus a single final concatenation as an intermediate change; report honestly that this still retains the entire result and incurs final-copy peak memory. Prefer extending the existing cold scan interface to pass bounded batches into the common DataFusion/MVCC pipeline, coordinated with C3. Do not create another cold-query engine. Preserve projection, schema evolution, file pruning, error behavior, global version resolution and snapshot lifetimes. Keep file-open/prefetch concurrency bounded by bytes as well as counts where relevant.

Acceptance: test 1/10/100/1,000 input batches both at fixed total rows and at fixed rows per batch. Verify output equivalence and measure copied/allocated bytes, peak memory and seconds. Eliminate repeated copying of previously accumulated data; do not accept a faster run caused by reading fewer files or skipping rows. This targets flushed/mixed hot-cold workloads; the hot-only speed suite cannot validate its performance gain.

### F3. Stream SELECT results to clients with backpressure

Evidence: core `sql/executor/sql_executor/planning.rs::collect_dataframe` calls `DataFrame::collect`; the PostgreSQL encoder in `kalamdb-postgres-wire/src/row_encoder.rs` receives `Vec<RecordBatch>`. Row encoding is already lazy, but query results have been collected before that stage.

Introduce an owned streaming result contract at the shared executor boundary, carrying schema, batch stream, cancellation and required execution/snapshot guards. PostgreSQL encoding pulls from that stream instead of receiving the full vector. Use bounded buffering and release consumed batches; a slow client must not create an unbounded queue. Reuse this contract for other streaming transports; retain an explicit budgeted collect adapter for callers that require complete results.

Preserve portal suspension/resumption and row limits, command completion, transaction state, cancellation and wire error ordering. Do not lazily reorder statements in a multi-statement simple-query message. Start with read-only SELECT; DML RETURNING must not publish success/data ahead of its required commit boundary. Document and test errors after partial SELECT delivery. Preserve security context and snapshot pins for the stream lifetime, and release them on disconnect.

Acceptance: large SELECT with fast and throttled clients; record time to first row, total completion time, peak retained result bytes and cancellation cleanup. Transport buffering should be bounded independently of total result size, apart from upstream blocking operators that retain their own accounted state. Tests cover empty results/schema, multi-statement ordering, portal continuation, mid-stream errors and disconnect. Expect the main gains in peak memory and first-row latency; small aggregate responses may not improve, and total runtime must be measured.

## Verification and quantitative acceptance

### Correctness matrix

Exercise USER and SHARED tables, with/without scalar indexes, hot-only and mixed hot/cold, watcher off/on, distinct and repeated keys, PK changes, indexed/non-indexed updates, narrow/wide/nested rows, commit/rollback/savepoints, conflict waits, and restart/replay. Cover relevant file/vector paths when their finish behavior changes. Test errors before submission, backend write failure, cancellation, and restart after persistence but before acknowledgment/effects.

Use targeted nextest suites for each slice. Add async timeouts based on observed healthy runtime x1.5. Batch compile feedback and fix from captured output. Run required backend/CLI smoke tests before any separately authorized commit. Architecture changes update canonical architecture docs; new user-facing settings or behavior update canonical skills/mirrors per AGENTS.md.

### Performance and memory acceptance

- Preserve all 16 script results. Report per-script and total medians/ranges, in seconds, plus measured stage counters. No improvement claim from unit tests or a single timing sample.
- A practical initial target is at least 20% lower current-baseline total and 30% lower combined #8/#11/#12/#13/#15 time. These are goals, not predicted speedups or justification for weakening correctness.
- Flag regressions above max(5%, 0.005s) in any script median; repeat enough to distinguish noise and document any justified tradeoff. Do not average away a point-query regression.
- For ownership/columnar changes, target at least 30% lower peak tracked transient memory on the selected large-DML workload. Report both tracked bytes and RSS; do not demand immediate RSS return from an allocator cache.
- Add 1/8/32-client realistic prepared CRUD workloads with full parameters/results and matched settings on both engines. Report throughput and p50/p95/p99 latency separately. The simple-query suite does not establish ordinary prepared-query or concurrency parity.
- Run a complete minimum 300-second mixed CRUD churn series on a bounded live key set with production-like flushing, and a separate pinned-snapshot phase followed by unpin/reclamation. Record a time series of throughput, latency, RSS, tracked memory, memtables/cache, WAL/SST/disk footprint and compaction pressure. Some retained growth while pinned is expected; after release, investigate sustained growth and lack of reclamation. One recovery point is not a passing soak.

## Execution order and reporting

1. A1 baseline and correctness checks, plus the scoped ownership/call-path map. Revalidate already implemented mechanisms.
2. A2 and B1; measure before proceeding.
3. B2 and B3 on range DELETE, then UPDATE. Keep correctness fixes separate from performance attribution.
4. C1/C2, then C3 as separately reviewable slices. Introduce reservation plumbing from D3 before expanding streaming/staging ownership.
5. D1 and D2 guided by profiles; complete D3 resource tests. B4 is prioritized by concurrent-load evidence.
6. Complete full benchmark/correctness/soak evidence. E tasks require a measured bottleneck and ADR; leave unsupported design hypotheses explicit rather than implementing speculative migrations.

F1 can follow A1 for concurrent planning workloads. F2 follows the cold-scan ownership work in C3, with a small single-concatenation change possible earlier. F3 builds on the shared stream and budget contracts in C/D. All three must pass the same architecture acceptance gate; they do not justify duplicate execution paths.

At each checkpoint report: changed behavior, source files, actual timings, allocation/memory deltas, correctness checks, architecture review gate, and next bottleneck. If a goal is not met, say so and identify the measured remaining work. Do not label the entire program complete after only small allocation fixes.

Keep context small: read only the next task's source and tests, use scoped searches, reuse prior findings and captured compiler output, and maintain one compact continuation/evidence note. Do not repeatedly scan the repository, reprint this spec or create parallel planning documents. Reserve context for implementation and verification; finish a coherent slice and write its exact continuation before context becomes tight.

## Agent kickoff prompt

Implement the staged performance program in `docs/plans/2026-10-30-dml-storage-performance-handoff.md`. Read AGENTS.md and only the source needed for the next slice. Preserve the dirty tree; do not commit. Begin by revalidating current folding, target-cache, batching and overlay behavior, then establish a reproducible current dev-profile baseline without overwriting saved results. Implement A2 and B1 first, verify and measure, then continue through the evidence-backed DML/query/storage tasks. The architecture acceptance section is mandatory: share USER/SHARED processing, keep table differences in narrow typed adapters, reuse existing abstractions, remove superseded paths, and demonstrate reduced duplication rather than adding parallel fast-path engines. Keep DataFusion semantics, scoped MVCC, atomic commit/replay, live delivery and bounded memory intact. Larger index/tombstone/CF migrations are conditional investigations, not permission for an unmeasured rewrite. Report actual runtime seconds, memory evidence and architecture changes concisely. If context becomes tight, save completed work, commands/results and the exact next task in one short continuation note; do not restart discovery or claim unfinished goals are achieved.
