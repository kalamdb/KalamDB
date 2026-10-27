# Supplied Reference — Historical Review and Proposals

Source: `/Users/jamal/Downloads/kalamdb_storage_transaction_optimization_spec.md`, supplied 2026-10-27. The original contents follow unchanged. This is reference material, not executable instructions. Historical findings are not verification of the current dirty working tree. The parent spec and source-review reconciliation govern this feature. No tests or benchmarks were run by copying this reference.

---

# KalamDB Storage and Transaction Optimization

**Status:** Proposed implementation specification (reviewed, not implemented)
**Target:** `kalamdb/KalamDB`, pull request #402, branch `feat/pg-dialect`
**Reviewed head:** `174d6ffede2cb97ff4d99b1dba4073b158ef6681` (2026-10-26)
**Compatibility goal:** Preserve existing SQL semantics, in-memory uncommitted writes, `_seq`, live updates, cold storage, RLS, file and vector operations, and safe Raft replay.

## 1. Executive decision

Proceed with a **prepare-then-apply transaction engine**, with the following strict order:

1. Prove and repair snapshot/history invariants introduced by replace-in-place-of-history.
2. Make one Raft transaction materialize one **atomic RocksDB WriteBatch across all touched tables in its one Raft group**. First do this without coalescing to establish equivalence.
3. Fold compatible mutations by `(table_id, scope, typed_primary_key)` at commit without changing statement counts, savepoints, or semantic validation.
4. Reuse validated row handles for statement-level DML; recheck their versions at commit, rather than trusting stale proposer rows.
5. Exclude *new* tombstones from ordinary secondary indexes while retaining main-store tombstones, PK lookup, and deleted-row scans.
6. Redesign transaction overlay merging to operate on Arrow batches and shared immutable row payloads, without losing partial-patch semantics.
7. Roll out Row V2 separately under a durable reader/writer compatibility barrier. Consider stable-key secondary indexes only as a separate migration.

Do **not** make the following unsafe shortcuts: (a) one WriteBatch per table instead of one per group; (b) skip rewriting `_seq`-keyed secondary index keys on UPDATE; (c) drop all mutation history without defining sync and repeatable-read guarantees; (d) use tombstone-only encoding where deleted-row values must remain available; (e) trust a scanned row as current without conflict checking; (f) remove intermediate staged operations before savepoint and SQL validation handling.

## 2. Code review: verified current behavior

Reviewed branch content (not a build/test run):

| Claim | Verified implementation | Qualification |
|---|---|---|
| Update/Delete transactions use one storage write per row | `kalamdb-core/src/applier/executor/dml.rs`, `apply_{shared,user}_transaction_batch_with_versions`: special fast path is only for all-INSERT; otherwise mutation loop calls provider writes individually | Ordinary, nontransactional multi-row UPDATE/DELETE **already** has provider batch methods |
| Replacement maintains all indexes, including tombstones | `kalamdb-store/src/indexed_store.rs`, `push_replace`, writes new main row and each extracted index key | `replace_batch[_async]` **already exists**; new API needed is append-to-a-shared-batch, not another per-table replacement API |
| Mutation data is duplicated in RAM | `kalamdb-transactions/src/write_set.rs`, `stage_with_estimated_size` and `staged_mutation.rs`, `overlay_entry` clone payload | Overlay maps use `Arc`/copy-on-write already; actual memory savings must be measured |
| Overlay materializes full rows | `kalamdb-tables/src/utils/base/provider.rs`, `base_scan_with_overlay` forces projection `None`; `overlay_exec.rs`, `record_batch_to_rows` and `json_rows_to_arrow_batch` | Current overlay handles partial patches and a residual filter. Naive filter/concat does not |
| Repeat row lookup | `shared_tables/provider/dml.rs` and `user_tables/provider/dml.rs` call `load_mutation_target` per PK in batch paths and when no `known_hot` is supplied | `known_hot` already optimizes repeated keys **within one transaction apply**, not scan-to-commit |
| Row V2 exists but is disabled | `kalamdb-serialization/src/object/envelope.rs`, `ROW_V2_WRITERS_ENABLED = false`; `row/{encode,decode}.rs` implement RowView and V2 | This flag is explicitly described as a format capability/upgrade gate; not simply a speed switch |
| `_seq` still anchors index keys | `kalamdb-store/src/index/prefix.rs`, `extract_key(... primary_key.prefix_index_seq())` | An UPDATE changes index key even if indexed column values are identical |
| Transactions limited to one Raft group | `RaftApplier::transaction_group_id` calls `RaftManager::resolve_transaction_group_id` | A group may still contain several tables; atomicity must cover all of them |

**Source anchors** (immutable revision):
- [Raft transactional DML](https://github.com/kalamdb/KalamDB/blob/174d6ffede2cb97ff4d99b1dba4073b158ef6681/backend/crates/kalamdb-core/src/applier/executor/dml.rs#L1356-L1604)
- [Indexed replacement and batching](https://github.com/kalamdb/KalamDB/blob/174d6ffede2cb97ff4d99b1dba4073b158ef6681/backend/crates/kalamdb-store/src/indexed_store.rs#L820-L914)
- [Write set](https://github.com/kalamdb/KalamDB/blob/174d6ffede2cb97ff4d99b1dba4073b158ef6681/backend/crates/kalamdb-transactions/src/write_set.rs#L18-L108)
- [Overlay execution](https://github.com/kalamdb/KalamDB/blob/174d6ffede2cb97ff4d99b1dba4073b158ef6681/backend/crates/kalamdb-transactions/src/overlay_exec.rs#L208-L403)
- [Provider overlay scan](https://github.com/kalamdb/KalamDB/blob/174d6ffede2cb97ff4d99b1dba4073b158ef6681/backend/crates/kalamdb-tables/src/utils/base/provider.rs#L501-L539)
- [V2 gate](https://github.com/kalamdb/KalamDB/blob/174d6ffede2cb97ff4d99b1dba4073b158ef6681/backend/crates/kalamdb-serialization/src/object/envelope.rs#L87-L113)

## 3. Required correctness invariants

**I1. No uncommitted durability or visibility.** DML statements stage into transaction-owned memory only. Other sessions never see these writes before successful COMMIT. Rollback and ROLLBACK TO SAVEPOINT leave no staged data in RocksDB.

**I2. Atomic group commit.** Any transaction accepted as belonging to one Raft group must expose all its table, index, and change-log mutations together or none. A per-table batch is not enough. Operations spanning multiple Raft groups remain rejected with a clear error until a distributed-transaction protocol is designed.

**I3. Deterministic replay.** Given the same Raft entry and prior state, every replica computes the same data operations and `_seq` values. Replay is idempotent; external side effects must not duplicate.

**I4. Snapshot correctness.** For a transaction pinned at version `S`, its reads must continue to show the version visible at `S`, even if another transaction commits a newer version `S+1`. **Critical suspected regression:** `push_replace` removes the older hot `_seq` key, and transaction scan compares new rows to snapshot bounds. Reproduce an update of a hot-only row by a concurrent transaction before shipping the optimization. If it fails, preserve old versions until safe (history store or properly pinned RocksDB snapshots with cold-tier snapshot coordination). Do not claim full MVCC if the storage model only keeps the latest row.

**I5. Uniqueness, RLS, and conflicts.** SQL constraints, PK uniqueness, FK checks, file references, RLS and write-write conflicts must still be correct. Commit must not silently overwrite a row that changed after its target was scanned. Treat PK renames as old-key removal plus new-key insertion, including collision checks.

**I6. Sync and live changes.** Deleted primary rows remain represented by a latest tombstone with full required payload and monotonically ordered `_seq`; normal live queries do not return deleted rows. If `_seq > since_seq` is promised as a **complete event stream**, an append-only history/change-log must preserve every committed event through the documented retention window. Keeping only the latest row and latest tombstone cannot satisfy complete history in the presence of repeated updates.

**I7. Side effects after commit.** Notifications, file deletion, vector updates, manifest changes and topic publishing cannot imply success before the atomic data write. File deletion must be scheduled after successful commit and be retryable/idempotent; manifest and vector state must be recoverable after crash. Any user-visible ACK means agreed durability and publication guarantees are met.

**I8. SQL behavior.** Preserve per-statement affected row counts and RETURNING semantics even when physical writes coalesce. Savepoints must still undo staged intermediate operations. No-op updates and inserts followed by deletes must follow defined SQL behavior without invented `_seq` events.

## 4. New transaction architecture

### Current hot path

```
BEGIN -> Stage mutations in Vec + cloned overlay -> COMMIT
      -> Raft entry -> per-row load/validate
      -> per-row store.replace_async -> per-row manifest mark
      -> accumulate notifications -> return
```

### Target hot path

```
BEGIN -> Stage original mutations + overlay in RAM
      -> At COMMIT: finalize valid mutations, optional semantic coalescing
      -> Propose 1 Raft entry to its one group
      -> State machine computes one PreparedTransaction:
           - validate all rows, constraints, conflicts, and versions
           - load each old PK once (cached by table+scope+PK)
           - resolve all final rows and full tombstones
           - append main, PK, secondary and history operations
           - buffer ordered notification / vector / file actions
      -> ONE backend.batch(all_operations) across touched group tables
      -> publish after-commit actions, update manifest once per table/scope
      -> ACK with original SQL row count and committed `_seq` frontier
```

**Atomicity scope:** Do not call `replace_batch_async` independently for each table. Introduce a pure prepare-to-operations API that aggregates all changes into a single underlying `StorageBackend::batch`. Every table in a transaction must share the compatible underlying backend/atomicity domain. Otherwise reject the transaction before any write.

### Proposed internal types (illustrative Rust signatures, adapt to existing generic types)

```rust
struct TargetIdentity { table_id: TableId, scope: Option<UserId>, pk: TypedPkKey }
struct PreparedRowMutation<K, V> {
    previous_hot: Option<(K, V)>,
    new_key: K,
    new_entity: V,
    expected_seq: Option<VersionId>,
}
struct PreparedTransaction {
    operations: Vec<Operation>,
    side_effects: CommitSideEffectPlan,
    original_affected_rows: usize,
    committed_seq: VersionId,
}

// In IndexedEntityStore: no I/O, no backend.batch, only generates ops.
fn append_replace_ops(
    &self,
    dst: &mut Vec<Operation>,
    previous: Option<(&K, &V)>,
    new_key: &K,
    new_entity: &V,
) -> Result<()>;

// Only the group-level commit owner writes:
backend.batch(prepared.operations)?;
// Then durable after-commit event outbox / post-commit dispatch.
```

Avoid code paths that use side effects during `prepare`; preparation must not alter RocksDB or delete files. Exact operation ordering must account for a repeated PK: old initial state -> final state should produce one replacement or an explicitly preserved event sequence, not several conflicting operations in the same WriteBatch.

## 5. Workstream A: single group-atomic batch for transactional UPDATE/DELETE (P0)

**Files to edit:**

1. `backend/crates/kalamdb-core/src/applier/executor/dml.rs`: rewrite `apply_shared_transaction_batch_with_versions` (about line 1356) and `apply_user_transaction_batch_with_versions` (about line 1070) to separate validate/prepare from one atomic apply. Keep pure-insert fast path only if it can participate in equivalent group-atomic semantics. Do not invoke `*_by_pk_value_deferred_with_hot` per row for the physical write.
2. `backend/crates/kalamdb-store/src/indexed_store.rs`: extract existing `push_replace` into a pure, reusable `append_replace_ops` and make current `replace`, `replace_batch` call it. Add an explicit policy hook for index treatment of deleted rows; do not hardcode generic index index 0 in system-table stores.
3. `backend/crates/kalamdb-tables/src/shared_tables/provider/dml.rs` and `user_tables/provider/dml.rs`: add `prepare_*` variants returning row operations and notifications/old rows. Reuse existing batch logic for materialization, schema coercion, not-null, cold lookup, and vector intentions without storage writes.
4. `backend/crates/kalamdb-core/src/applier/applier.rs` and Raft transaction state machine entry points: preserve routing to exactly one group; place group-wide batch ownership at the apply layer.
5. `backend/crates/kalamdb-core/src/transactions/commit_result.rs` plus manifest/notification/vector/file side-effect handling: dispatch only after the single successful batch; make retry/outbox or reconciliation explicit.

**Implementation sequence:**
- A1: Add counters `txn_mutations_input`, `txn_unique_pk_targets`, `txn_pk_loads`, `txn_rocksdb_batch_calls`, `txn_rocksdb_ops`, `txn_rocksdb_bytes`, `txn_prepare_us`, `txn_write_us` and failure-injection hook. Baseline original behavior.
- A2: Preserve existing `ordered_mutations` verbatim and build a correct transaction-level `PreparedTransaction` for mixed operations; first **do not coalesce** so tests can compare old/new semantics. Resolve repeated-key intermediate states entirely in memory. If historical events are required, append them into a separate history structure while writing only final hot state.
- A3: Validate all required conditions *before* `backend.batch`; append Operations for every touched table/index/history record to one vector; call once. No partial flush or chunked RocksDB commits. Apply configured maximum transaction bytes/rows and return a safe limit error rather than silently splitting atomic commits.
- A4: Move manifest marks to once per touched `(table_id, scope)` after successful write, or include durable pending state in atomic batch. Enqueue/file cleanup and vector effects after durable success, with replay safety.
- A5: Only after tests pass, enable commit coalescing (Workstream B).

**Failure conditions to cover:** validation error after 24,999 rows; storage batch error; crash before/after batch; stale base version; conflicting PK; vector/file operation failure after commit; one Raft entry replicated and replayed twice.

## 6. Workstream B: semantic PK coalescing (P0 after A)

**Files:** `backend/crates/kalamdb-transactions/src/write_set.rs`, new `commit_coalescer.rs` in that crate, `backend/crates/kalamdb-transactions/src/overlay.rs`, `backend/crates/kalamdb-core/src/transactions/coordinator.rs` (commit around line 346), `backend/crates/kalamdb-core/src/applier/executor/dml.rs` (prevalidation and replay).


**Rule:** Keep the original ordered mutation history in memory until savepoints are resolved and SQL statement validations have run. At COMMIT, derive a *final-state list* ordered deterministically by surviving mutation order. Key identity must be typed/canonical and scoped by table and user. Do not use a formatted `table|pk` string as a collision-proof identity.

| Earlier staged state | Next operation | Final candidate | Requirements |
|---|---|---|---|
| Missing | INSERT | INSERT | Validate PK was absent at transaction snapshot and is still legal at commit |
| INSERT | UPDATE | INSERT with merged final columns | Preserve untouched inserted fields; SQL UPDATE row count still recorded |
| UPDATE | UPDATE | UPDATE with merged patches | Last assigned value wins, including explicit SQL NULL |
| INSERT | DELETE | No final hot row (if no preexisting row) | No tombstone/event unless documented event-stream semantics require these transitions |
| UPDATE | DELETE | DELETE with full last-before-delete row | Preserve full deleted payload; record appropriate `_seq` |
| DELETE | INSERT | Reinsert / resurrection | Validate existing base history and uniqueness; treat as new live state with correct PK semantics |
| INSERT | INSERT | Error unless SQL ON CONFLICT explicitly resolves | Do **not** silently coalesce away a duplicate PK violation |
| DELETE | UPDATE | Error or zero affected, matching SQL semantics | Never invent a partial row after delete |
| DELETE | DELETE | One final delete (or no additional effect) | Preserve original affected-row counts per statement |

**Additional cases:** row inserted then deleted then reinserted; PK rename old and new identities; ON CONFLICT/UPSERT; DEFAULT and generated columns; triggers; FILE column replacement; unique secondary keys; same table across different USER scopes; rolled-back savepoints; NULL versus column-not-present in partial update. Coalescing only physical changes is safe if the SQL layer has already evaluated/validated all intermediate statements and needed trigger side effects.

**Version assignment:** Avoid allocating `_seq` in a way that changes after collapsing mutations or depends on HashMap iteration order. Define a deterministic mapping of final visible transitions to `(raft_log_index, stable_ordinal)` and document whether skipped ordinals are allowed. If the public change stream has per-statement events, retain ordered events separately from the coalesced latest-state row. The `affected_rows` returned by `TransactionCoordinator::commit` remains the original SQL-affected count, not the final unique-PK count.

**Memory representation:** Eventually store `Arc<Row>` or an ordinal row plus changed-column bitset; have overlay entries hold references/handles into an immutable write-set snapshot instead of cloning column-name strings and ScalarValues. Existing `Arc`/COW overlay maps should be retained for execution-plan isolation. Estimate/write-set memory accounting and savepoint truncation must count unique retained allocations correctly.

## 7. Workstream C: remove DML scan-to-commit double lookups (P0/P1)

**Files:**
- `backend/crates/kalamdb-tables/src/utils/datafusion_dml.rs`: change DML collect helpers to optionally return typed targets, including PK, full previous row, source (hot/cold), expected `_seq`, and target scope. Existing `collect_matching_rows_with_projection` is a starting point.
- `backend/crates/kalamdb-tables/src/shared_tables/provider/scan.rs` and `user_tables/provider/scan.rs`: offer an internal scan path returning the resolved original version along with the projected SQL row; preserve RLS and cold/hot winner resolution.
- `backend/crates/kalamdb-tables/src/shared_tables/provider/dml.rs` and `user_tables/provider/dml.rs`: accept `KnownTarget` or per-statement target lists in their prepare methods. Existing `..._deferred_with_hot` already accepts a cached row *inside* one apply, so extend the pattern without exposing unchecked stale rows.
- `backend/crates/kalamdb-core/src/applier/executor/dml.rs`: load once per distinct `(table,scope,PK)` for new targets; cache across all mutations; validate the expected version at commit/apply.
- `backend/crates/kalamdb-transactions/src/staged_mutation.rs`: only add an optional expected version or private non-Raft local target handle if needed. Avoid transmitting full preimage rows over Raft just to eliminate a read.

**Proposed internal interface:**

```rust
struct KnownMutationTarget<K, V> {
    pk: TypedPkKey,
    prior_hot: Option<(K, V)>,
    prior_cold: Option<V>,
    expected_seq: VersionId,
    schema_version: u16,
}

// Internal to leader execution, not automatically a Raft payload:
fn prepare_update(target: &KnownMutationTarget<_, _>, patch: &Row, next_seq: VersionId)
    -> Result<PreparedRowMutation<_, _>>;
```

**Important concurrency boundary:** A row scanned before Raft commit may no longer be current. Do not blindly write the scanned preimage. Either (1) verify the scan's `expected_seq` at Raft apply and reject/retry according to transaction isolation, or (2) resolve authoritative current rows exactly once during apply, with a per-transaction target cache. The latter is safer for deterministic replay and may already remove most redundant *apply-time* reads. First optimize one-point PK reads and `multi_get`/batch-prefetch where supported; only then try passing scan handles across layers.

**Cold rows:** The `Cold` mutation-target case has no old hot key to delete but must still mask the older Parquet version and produce complete UPDATE/DELETE rows. A hot tombstone must prevent resurrection from Parquet on later reads.

**RLS/constraints:** Row handles are not authorization tokens; check applicable RLS and FK/unique rules at the proper SQL/commit boundary. The scan only identifies candidate rows.

## 8. Workstream D: tombstones in main and PK index, not secondary (P1)

**Files:**
- `backend/crates/kalamdb-store/src/indexed_store.rs`: invoke `extract_key` for all old index entries and delete them. For the *new* deleted row, write the main row and PK index only. Implement via a per-index semantic policy/role; this store is generic and must not assume all index arrays have PK at element 0.
- `backend/crates/kalamdb-tables/src/common.rs`: the table-index factory (`table_prefix_indexes` / `new_indexed_store_with_pk`) must mark the PK lookup role vs ordinary secondary indexes, or wrap the row-specific index definitions.
- `backend/crates/kalamdb-tables/src/shared_tables/pk_index.rs` and `user_tables/pk_index.rs`: verify indexed deleted-row PK remains searchable and latest-version lookup returns tombstone.
- `backend/crates/kalamdb-tables/src/utils/base/deferred.rs`, `shared_tables/provider/scan.rs`, and `user_tables/provider/scan.rs`: when `keep_deleted` is true (`_deleted` predicate or `_seq` sync), do **not** select a secondary-index-only scan that excludes tombstones. In shared scan the existing guard `if since_seq.is_none()` is not sufficient; use a semantic guard such as `!keep_deleted` and ensure snapshot visibility.
- `backend/crates/kalamdb-tables/src/utils/base/hot_scan.rs`: keep any current-row index fast path correct for stale/old tombstone entries during migration.
- SQL planner/optimizer and table index metadata: ensure partial indexes or covering indexes do not accidentally claim full-table coverage of deleted rows.

**Desired write shape:**

```
Before: hot(seq=10, deleted=false), PK(id,10), idx_a(a,10), idx_b(b,10)
Delete: remove old hot(10), PK(id,10), idx_a(a,10), idx_b(b,10)
        write hot(seq=11, deleted=true, full_row), PK(id,11)
        do not write idx_a(a,11) or idx_b(b,11)
```

**Version/cold caveat:** Whether to remove `hot(seq=10)` depends on the snapshot/history design in I4 and event-stream contract I6. The diagram describes current replacement behavior, **not permission to discard needed versions**.

**Query rules:** Normal `WHERE a=7` scans may use `idx_a`; `WHERE _deleted=true`, `WHERE a=7 AND _deleted=true`, or `_seq > cursor` must use the authoritative main/PK/history path and apply the residual predicate. Primary-key lookup must observe tombstone and not fall back to an older cold value.

**Migration:** New write policy alone leaves historical tombstone keys in existing secondary indexes. Add an `index_layout_version`/generation marker and an online secondary-index rebuild or opportunistic cleanup. During migration, scanners must tolerate both legacy and new contents without missing deleted rows or returning stale live results. Rebuild under snapshot + catch-up change log/watermark, atomically swap generation, verify counts. Do not require an offline full database rebuild if online transition is feasible.

**Tombstone payload:** Keep full deleted values for the user-requested live-update and sync contracts. The implemented `encode_tombstone_row_v2` is *not* a valid replacement for full deleted-row values unless those values are captured durably in another independently queryable change log.

## 9. Workstream E: Arrow-native transaction overlays (P1)

**Files:** `backend/crates/kalamdb-transactions/src/overlay_exec.rs`, `overlay.rs`, `write_set.rs`, `staged_mutation.rs`; `backend/crates/kalamdb-tables/src/utils/base/provider.rs` (`base_scan_with_overlay`); `backend/crates/kalamdb-tables/src/utils/datafusion_dml.rs` (projection selection/row materialization).

**Phase E1 - no scalar map conversion for the scanned base:**
- Extract typed PK array/column from each input `RecordBatch` (keep original Arrow buffers); build a bitmap for rows replaced or deleted by the overlay.
- `filter_record_batch` the base by that bitmap **without turning the other columns into ScalarValues**.
- Build fully materialized replacement rows in Arrow builders: for an UPDATE with a partial patch, apply the patch to its original base row using `take`/array builders or a compact typed row accessor; do not append the patch with nulls in untouched columns.
- Build inserted rows into Arrow once, align types/schema, and concatenate/stream as needed. Apply the **complete residual WHERE predicate after overlay**; then final projection and an appropriate LIMIT/ORDER BY stage.
- Keep bounded batches/partitions rather than collecting every committed row into one Vec. Preserve DataFusion `ExecutionPlan` properties; make clear which ordering and partitioning guarantees are lost.

**Phase E2 - avoid full-column scans:**
- Compute required physical columns as `{requested projection} union {PK} union {filter dependencies} union {columns needed by pending patches} union {RLS dependencies}`.
- For a PATCH that needs unmodified columns, selectively materialize the base row by PK, not all table columns for all rows.
- If a staged UPDATE changes the column referenced by a pushed-down filter, a committed-row index scan alone is not complete. Disable unsafe pushdown or add an explicit delta side scan of affected PKs. Both directions must work: old value no longer matches and new value matches, and vice versa.
- Never apply LIMIT before overlays and residual filtering in a way that hides newly matching rows. Keep correct predicates for joins, aggregates, functions, and order-dependent operations.

**Phase E3 - memory:** Use `Arc<Row>` first as a low-risk de-duplication improvement. Only move to `Arc<[ScalarValue]>`/ordinal payloads when column identity, defaults, schema evolution, and *explicit NULL vs absent field* can be preserved with changed-field bitmasks. Retain savepoint rewind and snapshot overlay behavior. Measure peak RSS and allocations; do not assume an exact 50% saving.

**Suggested regression query:**

```sql
BEGIN;
UPDATE items SET score = 9 WHERE id = 1;       -- original score was 1
SELECT id, name FROM items WHERE score = 9;   -- must include id=1, complete name
SELECT id FROM items WHERE score = 1;         -- must exclude id=1
ROLLBACK;
```

## 10. Workstream F: Row V2 rollout and secondary-index V2 (P2/P3)

**Row V2 (independent rollout):**
- `backend/crates/kalamdb-serialization/src/object/envelope.rs`: preserve `set_row_v2_writer_enabled` as a test/control hook, but gate production enablement on a **durably agreed cluster-wide reader capability** rather than a local process flag alone.
- `backend/crates/kalamdb-serialization/src/row/encode.rs` and `row/decode.rs`: test mixed V1/V2 rows, null bitmaps, variable columns, schema evolution, deleted payloads, and the `RowView` projected decode path. Use the existing V2 implementation; avoid rewriting the format.
- `backend/crates/kalamdb-tables/src/row_codec.rs`: verify table codecs honor writer gate and historical schema definitions on all relevant paths, including bulk writes, flush and restart.
- Raft startup/join/capability logic: before enabling new writes, every voting or soon-to-be-voting replica must be able to read V2 (and must not downlevel write). Persist format floor so restart cannot silently revert writer mode while V2 files exist. A mixed-binary cluster must not produce unreadable Raft-applied rows.
- Enable behind a rollout stage: reader support -> upgrade all nodes -> durability/capability barrier -> V2 writer enable. Add kill switch only if returning to V1 writer keeps all existing V2 rows readable; do not promise downgrade to an old reader.
- Benchmark encoding+decoding CPU, projected access, hot-store bytes and RocksDB compaction bytes; V2 is not guaranteed faster for every row shape.

**Secondary-key redesign (different storage migration):**
- Existing index `key=(scope,indexed_values,_seq)`, `value=_seq` necessarily changes on every UPDATE.
- Proposed V2: `key=(scope,indexed_values,canonical_typed_pk)`, `value=current_seq` or empty; reader resolves PK->current hot row and checks current index version. If `_seq` changes but indexed values do not, only update `value` (or use a current-PK indirection) while avoiding delete+insert of the key.
- To avoid **two RocksDB lookups** for each secondary hit, consider a compact covering row handle or primary-key cache; benchmark the index lookup cost against write savings. Composite/range indexes require correct collation, NULL handling, and schema evolution.
- Index V2 requires separate `index_layout_version`, dual-read or online shadow-build, transaction-synchronized catch-up, atomic swap, and consistency validation. Do not enable alongside the basic transaction batching PR.

## 11. Performance work beyond the original five

**Bulk PK validation:** Existing `prevalidate_shared_transaction_batch` checks insert PK existence one-by-one. Group typed keys by table/scope; deduplicate and use batched `multi_get` or efficient prefix probes when supported. Maintain proper uniqueness against both already-committed and staged inserts, tombstones, and cold Parquet. Avoid assuming prefix existence alone establishes a live row (tombstones still indexed by PK).

**Reduce thread scheduling and round trips:** Profile `spawn_blocking` invocations, per-row schema coercion, per-row provider lookups, notification checks and allocations. Group by provider and scope, cache schema/PK parser and `has_subscribers`, reserve vectors and buffers, encode with reusable thread-local scratch. A RocksDB batch removes per-row scheduling overhead but Raft serialization and DML expression evaluation may remain dominant.

**Direct multi-row DML:** Existing nontransactional provider `delete_batch_by_pk_values_deferred` and `update_batch_by_pk_values_deferred` are already batch writers; optimize their *per-PK target loading* and unify them with the new prepare/apply APIs. Do not regress these fast paths.

**Two distinct read APIs:** Use optimized latest-PK point lookup for CRUD and an explicit history/sync scan path for retained `_seq` changes. Avoid forcing every ordinary SELECT into an event-history scan.

## 12. Tests (mandatory before merge)

### Unit and component tests
- `kalamdb-store/src/indexed_store.rs`: operation lists for insert/update/delete, same-key replacement, changed/unchanged index keys, PK tombstone retention, secondary tombstone omission, composite/scoped keys, and atomic batch failures.
- `kalamdb-transactions/src/write_set.rs` and new coalescer module: every transition in section 6; intermediate statement counts, savepoints, partial patches, explicit NULL, typed PK equality, USER scopes, PK renames.
- `kalamdb-transactions/src/overlay_exec.rs`: Arrow-native equivalence to row-map implementation for inserts/updates/deletes, filter newly matches/stops matching, partial update, NULL, projection, limit, RLS and empty batches.
- `kalamdb-serialization/src/row/*`: V1/V2 round-trip for all supported data types; projected decode; old reader/writer capability gate tests.

### Integration and durability tests
1. **Atomic 25,000-row UPDATE:** one physical RocksDB batch for one Raft commit, all index values correct, rows affected 25,000.
2. **Atomic 25,000-row DELETE:** one batch, 25,000 PK tombstones with full payload, no new ordinary secondary tombstone keys.
3. **Mixed multi-table same-group transaction:** UPDATE table A and DELETE table B, inject validation failure on final mutation; neither table changes. Success path commits both together.
4. **Different-group transaction:** rejected before writing; no partial data.
5. **Snapshot regression:** tx A reads `(id=1, value=old)` at snapshot S; tx B commits UPDATE to `(new)` at S+1; tx A rereads and must still see `old`. Repeat with hot-only, mixed cold, and post-flush data. This is a **release blocker**.
6. **Write conflict:** tx A scanned seq N, tx B changed the same PK to N+1; tx A cannot overwrite unseen changes silently.
7. **Rollback/savepoint:** INSERT -> UPDATE -> SAVEPOINT -> DELETE -> ROLLBACK TO SAVEPOINT -> COMMIT; final state and row counts correct. Rollback without commit writes nothing.
8. **Cold masking:** old row in Parquet; DELETE creates hot tombstone; PK GET and secondary scans do not resurrect the cold row.
9. **Deleted query:** `SELECT ... WHERE _deleted=true` and `WHERE indexed_col=x AND _deleted=true` return tombstone row with old values, regardless of secondary-index policy.
10. **Sync cursor:** `SELECT ... WHERE _seq > cursor` returns required tombstone and obeys documented semantics for multiple updates; tests distinguish *latest state since cursor* vs *all events since cursor*.
11. **Live notifications:** correct final old/new/full delete payload, ordering by version, exactly once as promised after Raft replay and reconnect.
12. **Crash and failover:** kill after prepare, before batch, after batch before side effects, and before ACK. Recover to either entire committed transaction or no transaction; post-commit work reconciles safely.
13. **Row V2 mixed cluster:** reads old and new formats after restart, refuses writer activation while an old-reader replica is eligible, preserves full tombstones.
14. **Index migration:** old tombstone index entries present, new writes omit; background rebuild with concurrent writes and failover yields correct query results.
15. **RLS/constraints/PK rename:** authorized and denied UPDATE/DELETE, duplicate insert, UPSERT, foreign-key integrity and unique index changes remain valid.

### Required test execution
- Run targeted crate unit tests for `kalamdb-store`, `kalamdb-transactions`, `kalamdb-tables`, `kalamdb-serialization`, `kalamdb-core` and Raft/state-machine components.
- Run all currently maintained 0.8 transaction/storage/SQL integration tests; check their names in branch Cargo configuration, not on `main`.
- Run SQL compatibility tests and full existing CI workflow on branch; use fault injection in a dedicated CI job.
- Run the same benchmark SQL files against the same release/branch build and the same SQLite disk/memory settings.

No tests or benchmarks were executed for this review; all status above is verified source inspection or proposed acceptance tests.

## 13. Benchmarks and acceptance criteria

Use instrumented Release builds, warm and cold cache, fixed schema/data cardinality, same disk and durability settings, at least 5 repeats and p50/p95/p99 where applicable. Break down client latency, Raft proposal/replication, statement planning, DML scan, PK lookups, row encode, RocksDB batch, manifest/notification and client transport.

| Scenario | Baseline to capture | Main expected outcome |
|---|---|---|
| 25k INSERT in one transaction | total, apply, PK validation, Raft bytes | Keep existing fast path, reduce redundant PK checks |
| 25k UPDATE in one transaction | total, apply, batch count, PK loads | Exactly **one** backend batch per one-group commit; no added lookups |
| 25k DELETE in one transaction | same plus index writes | One group batch; secondary tombstone Put operations zero |
| 25k updates to the same PK | staged count, final physical ops, log bytes | Physical latest-state operations O(1) per distinct PK; statement semantics remain |
| 100/1000 SELECT in dirty transaction | CPU, allocation, bytes scanned | No full-table Arrow-to-Row-to-Arrow conversion in optimized overlay |
| UPDATE of unindexed col | index mutations/row | Do not promise improvement until stable-key index V2 migration |
| `_deleted` filters and `_seq` sync | missed/duplicate values, latency | Zero correctness regressions, possibly more expensive authoritative path |
| Row V1 versus V2 mixed sizes | encode/decode us, bytes/row, RSS | Only enable V2 if compatible and demonstrably beneficial |

**Hard gates:** zero correctness failures; one atomic `backend.batch` per supported transaction (except explicitly optimized atomic alternatives); zero partial writes on fault injection; no duplicate post-commit effects after replay; no snapshot regression; no dead-row secondary writes after migration; maintained sync contract. Performance target: achieve a meaningful, reproducible improvement versus branch baseline, without imposing an arbitrary "10x" claim. Track memory reductions as measured values, not an assumed half.

## 14. Rollout and implementation order

- **Phase 0: correctness and observability**. Add snapshot regression and sync history tests; settle guarantees, instrument original per-row commit, identify physical batch/failure points. Fix snapshot/history if it fails.
- **Phase 1: atomically prepared transaction**. Unify provider prepare operations, group-wide RocksDB batch, after-commit side effects and failure-injection tests. Keep old mutation semantics initially.
- **Phase 2: coalesce at commit**. Add deterministic state-machine fold and scope-aware typed identity; preserve statement counters, constraints, savepoints and full event history (if promised). Benchmark repeated-PK payloads and Raft log shrinkage.
- **Phase 3: skip deleted secondary index entries**. Introduce index role and scan planning safeguards, mixed-version index reading and shadow index rebuild. Full deleted row stays main/PK.
- **Phase 4: Arrow-native overlay and projection**. Keep baseline fallback behind feature gate; test predicate movement and partial updates.
- **Phase 5: Row V2 production enablement**. Cluster-wide reader readiness, durable format floor, mixed-format tests, benchmarks.
- **Phase 6: optional stable-PK secondary index layout V2**. Independent migration/spec and benchmark; do not conflate with Row V2.

Suggested feature controls: `txn_atomic_batch`, `txn_commit_coalesce`, `tombstones_pk_only`, `txn_arrow_overlay`, `row_v2_writer`, `secondary_index_layout_v2`. **Storage and Raft determinism caveat:** data-format and replay-affecting gates must be consensus/cluster-capability controlled, not divergent local booleans.

## 15. Execution task list

- [ ] T001: Add repeatable-read hot-only and cold-tier regression tests (release blocker).
- [ ] T002: Define current-state vs complete-event `_seq` sync contract and retention/watermark behavior.
- [ ] T003: Instrument per-row commit, reads, index writes, Raft bytes and side effects; capture baseline benchmarks.
- [ ] T004: Create pure append-replacement-operation API in `IndexedEntityStore` and migrate existing single/batch callers to it.
- [ ] T005: Implement provider-level `PreparedMutation` building for both shared and user tables without I/O side effects.
- [ ] T006: Aggregate all table operations in one Raft group into one underlying `backend.batch` and enforce transaction size/backend domain limits.
- [ ] T007: Make post-commit file/vector/manifest/notification actions recoverable and idempotent; add failpoints.
- [ ] T008: Add atomicity/replay/conflict tests for same-group mixed-table transactions and rejection for cross-group transactions.
- [ ] T009: Add commit coalescer with typed scoped PK, deterministic ordering and full transition table; do not alter SQL result counts.
- [ ] T010: Optimize bulk PK prevalidation/prefetch, reusing authoritative rows and expected version checks.
- [ ] T011: Introduce index semantic role and omit new tombstones from secondary indexes; preserve main/PK full rows.
- [ ] T012: Disable secondary-only scans for deleted-row and sync queries; add legacy-index migration/rebuild.
- [ ] T013: Add Arrow-native batch overlay with correct full-row patching and residual filtering.
- [ ] T014: Narrow overlay scan projections and handle updated-filter-column index pushdown safely.
- [ ] T015: Reduce overlay value clones via shared row payload handles, preserving savepoint rewinds and NULL semantics.
- [ ] T016: Enable Row V2 only after persisted cluster reader capability barrier and mixed-version correctness tests.
- [ ] T017: Write independent stable-key secondary-index V2 design/migration if metrics justify it.
- [ ] T018: Run complete benchmark suite (including client/server split), compare correctness and p50/p95, publish report and flamegraphs.
- [ ] T019: Run full CI plus fault-injection/upgrade suite on the feature branch before merge.

## 16. Definition of done

The optimization is done only when (1) the correctness invariants pass, (2) transaction commit uses one atomic storage batch across all tables in its Raft group, (3) `_seq` and snapshot/live/deleted-row semantics are documented and tested, (4) no required FILE/vector/notification side effect is lost or duplicated, (5) the benchmark suite reproduces improvements on the same build/hardware, and (6) the rollout can be safely enabled on existing databases, including mixed V1/V2 rows and old secondary-index contents.
