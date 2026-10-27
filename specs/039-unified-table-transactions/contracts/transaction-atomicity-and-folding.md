# Contract: Atomic Transaction Commit, Commit Folding, and Columnar Overlay

**Feature**: `039-unified-table-transactions`  
**Covers**: FR-005–FR-018, SC-001–SC-004, SC-010

---

## 1. Group-Wide Atomic Commit Contract (FR-005–FR-012)

### Preconditions
1. All mutations in a transaction belong to a single compatible commit domain (`GroupId::Data(UserShard(_))` or `GroupId::Data(SharedShard(_))`), enforced both at `TransactionCoordinator::stage_batch` and `DmlExecutor` commit preparation (`FR-008`).
2. Total folded version slots `<= 65,536` (`ADR-022` ordinal limit) and staged buffer bytes `<= max_transaction_buffer_bytes` (`FR-008`).
3. Leadership for the bound `GroupId` is current at commit entry and verified through Raft proposal application.

### Execution Phases
1. **Fold & Order**:
   - Fold `TransactionWriteSet` into `FoldedCommitPlan` (once uncoalesced atomic baseline is verified, or directly in uncoalesced mode for baseline comparison).
   - If `FoldedCommitPlan` has zero durable storage/history/effect mutations, transition transaction to `Committed` with `0` storage submissions and return original statement-level `affected_rows`.
2. **Authoritative Target Load & Validation**:
   - Batch-load each distinct `ScopedTargetKey` once via `TargetResolutionCache`.
   - Validate `FILE` references, `NOT NULL`, foreign keys, PK uniqueness (against hot index, cold manifest/Parquet, and in-batch peers), and write-conflict version bounds (`committed_version <= snapshot_bound`).
   - Any error aborts the transaction cleanly before any storage write or external side effect.
3. **Single Atomic Batch Submission**:
   - Construct one `Vec<kalamdb_store::Operation>` across all touched tables, PK indexes, secondary indexes, and pending-manifest index entries.
   - Submit via a single `StorageBackend::batch(ops)` call.
4. **Post-Persist Side Effects**:
   - Executed strictly after `StorageBackend::batch` succeeds:
     1. `CommitSequenceTracker::observe_committed(group_id, log_index)`
     2. In-memory `ManifestService` pending state updates
     3. Idempotent vector hot-store batch staging
     4. Topic publisher events and live query `ChangeNotification` fanout
     5. Best-effort `FileRef` deletions for overwritten/deleted file columns

---

## 2. Commit-Fold Equivalence & History Contract (FR-013, FR-014, FR-018)

1. **Statement-Time vs Commit-Time Accounting**:
   - Statement execution (`INSERT`, `UPDATE`, `DELETE`) returns the immediate PostgreSQL-compatible affected row count for that statement.
   - Savepoints (`SAVEPOINT`, `ROLLBACK TO SAVEPOINT`, `RELEASE SAVEPOINT`) operate on the statement-ordered mutation log and restore exact intermediate overlay state.
2. **Latest-State vs Complete-Event Replay (FR-018)**:
   - Latest-state queries and `VersionedTableEngine` hot/cold state receive the folded final state per `ScopedTargetKey`.
   - When a table has active complete-event subscribers/topics or retained change-log requirements, promised intermediate transitions (or explicit resynchronization/gap signals when history expires or is unavailable) are preserved in domain order.

---

## 3. Columnar Transaction Overlay Contract (FR-016, FR-017, SC-010)

### Scan Planning (`base_scan_with_overlay`)
- Given caller `projection: Option<&Vec<usize>>` and `filters: &[Expr]`:
  - Compute `required_columns = projection ∪ expr_columns(filters) ∪ {pk_column}`.
  - Pass `Some(required_columns)` to `base_scan` instead of `None` so cold Parquet and hot KOBJ selected decode only materialize needed columns.

### Batch Execution (`TransactionOverlayExec`)
1. **Disjoint Batch Fast Path**:
   - Extract `PkBucketKey`s from the child `RecordBatch`'s PK column array using vectorized/typed array access (`pk_bucket_key_from_array`).
   - If no `PkBucketKey` in the batch is present in `overlay.deleted_keys` or `overlay.updated_keys`:
     - Keep the child `RecordBatch` as-is without converting to `Vec<Row>`.
2. **Intersecting Batch Path**:
   - Apply deletes via Arrow `filter_record_batch` boolean mask.
   - Apply partial updates by patching only the modified columns on matching rows, preserving untouched columns and base `_version`.
3. **Appended Inserts**:
   - Convert `overlay.inserted_keys` for the target scope once into an Arrow `RecordBatch` matching `input_schema` (with `_version = NULL` and `_deleted = false`), filter out any PKs already handled, concatenate batches, apply `residual_filter` (`PhysicalExpr::evaluate` + `filter_record_batch`), apply `fetch`, and project to `final_projection`.
