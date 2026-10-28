# Tasks: Unified Table Processing and Atomic Transaction Optimization

**Input**: Design documents from `/specs/039-unified-table-transactions/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Included. FR-035, the story independent tests, and `quickstart.md` require them. Write each story's tests so they fail before the implementation task they guard. Use the default `dev` profile. Report runtimes in seconds.

**Branch**: Stay on `feat/pg-dialect` (PR #402). Do not open a new branch.

**Organization**: Tasks follow user stories. Story 2 is sequenced before Story 1 because the atomic commit in Story 1 is implemented on `VersionedTableEngine` from Story 2. Both are P0. The MVP is Story 2 plus Story 1.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on an incomplete task)
- **[Story]**: User story label (`[US1]` … `[US7]`)
- Setup, Foundational, and Polish tasks have no story label

## Path Conventions

Backend crates live under `backend/crates/`. Evidence goes in `specs/039-unified-table-transactions/baselines.md`. Architecture notes go in `docs/architecture/`.

---

## Phase 1: Setup (Characterization)

**Purpose**: Record the pre-change evidence that later gates compare against (FR-036, SC-009, SC-015).

- [X] T001 Create `specs/039-unified-table-transactions/baselines.md` with empty result tables for consolidation LOC, `StorageBackend::batch` counts, 25,000-row insert/update/delete timings, direct system point/index lookup latency, and one `pg_catalog` plus `information_schema` introspection query. Columns: workload, build profile (`dev`), hardware note, median seconds, p99 seconds, batch submissions.
- [X] T002 Fill the pre-change rows in `specs/039-unified-table-transactions/baselines.md` by running the LOC script and Gate 2 timing steps in `specs/039-unified-table-transactions/quickstart.md` before any provider edit. Include direct lookups of `system.users`, `system.jobs`, namespaces, and types, plus the wire introspection query (depends on T001).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Crate-boundary moves and the schema macro that every later story consumes (FR-033, FR-046, FR-047).

**Checkpoint**: `kalamdb-system` and `kalamdb-views` still do not depend on `kalamdb-tables`. `cargo check -p kalamdb-publisher -p kalamdb-transfer -p kalamdb-datafusion-sources -p kalamdb-macros` passes.

- [X] T003 [P] Move `TopicMessageStore`, `TopicMessage`, and `topic_message_schema` from `backend/crates/kalamdb-tables/src/topics/` to `backend/crates/kalamdb-publisher/src/storage/`, update callers, and remove the `kalamdb-tables` dependency from `backend/crates/kalamdb-publisher/Cargo.toml`.
- [X] T004 [P] Add `TransferError` in `backend/crates/kalamdb-transfer/src/`, switch `backend/crates/kalamdb-jobs/src/executors/table_transfer.rs` and `backend/crates/kalamdb-jobs/src/executors/user_export.rs` to it, and remove `kalamdb-core` from `backend/crates/kalamdb-transfer/Cargo.toml`.
- [X] T005 [P] Move `rows_affected_plan`, `dml_scan_projection`, `projected_schema`, `PreparedUpdateAssignments`, and `collect_input_rows` from `backend/crates/kalamdb-tables/src/utils/datafusion_dml.rs` into `backend/crates/kalamdb-datafusion-sources/src/dml.rs`, and point existing callers at that module.
- [X] T006 [P] Extend `#[table]` in `backend/crates/kalamdb-macros/src/lib.rs` so it still emits `definition() -> TableDefinition` and also emits a `OnceLock` `arrow_schema()` from `definition().to_arrow_schema()`, the primary-key column name, and ordinal column order. Do not add a second schema language.

---

## Phase 3: User Story 2 - One provider boundary for every table kind (Priority: P0)

**Goal**: User, shared, stream, and persisted system tables share one index implementation, one hot/cold path, and one schema-history decoder. `KalamTableProvider` serves user, shared, and stream SQL. Persisted `system.*` SQL reads use `KeyedTableProvider`, and the existing direct `get`/`put`/`scan_index`/`update_with_old`/`delete` API stays off DataFusion. `pg_catalog` projections stay as written. Shared-table row-level security stays on `SharedTableAuthorization`. A later view over a shared table uses that provider and that security. Optimized function bodies are moved or called, not rewritten (FR-001–FR-004, FR-038–FR-047, SC-005, SC-013, SC-015).

**Independent Test**: One scan/DML suite across user, shared, and stream; SQL DML rejected on persisted system tables while direct `get`/`put` still works; shared RLS still filters reads and rejects failed `WITH CHECK` writes; `pg_catalog` and `information_schema` introspection matches today's results with unchanged `compute_batch` bodies; SC-015 median and p99 are ≤ the T002 baseline; `build_record_batch!` remains; hand-written duplicate schemas and per-relation provider copies are gone.

### Tests for User Story 2

- [X] T007 [P] [US2] Add failing parity tests that user and shared tables both execute `columnar_hot_scan`, `sum_hot_covering_integers`, and `plan_literal_equality_updates` in `backend/crates/kalamdb-tables/tests/provider_source_models.rs`, `backend/crates/kalamdb-core/src/sql/executor/sql_executor/covering_aggregate.rs` tests, and `backend/crates/kalamdb-core/src/sql/executor/sql_executor/equality_update.rs` tests.
- [X] T008 [P] [US2] Add failing tests that `get_user_by_id`, job status index scans, namespace lookup, and catalog type lookup call `IndexedEntityStore` directly and do not build a DataFusion plan, in `backend/crates/kalamdb-system/src/providers/users/users_provider.rs`, `backend/crates/kalamdb-system/src/providers/jobs/jobs_provider.rs`, `backend/crates/kalamdb-system/src/providers/schemas/namespaces_provider.rs`, and `backend/crates/kalamdb-system/src/providers/catalog/catalog_tests.rs`.
- [X] T009 [P] [US2] Add failing tests that `pg_class`, `pg_attribute`, `pg_namespace`, and the Kalam `information_schema.columns` extension take column order from `#[table]` and still satisfy the existing shim queries in `backend/crates/kalamdb-views/tests/pg_catalog_slice.rs` and `backend/crates/kalamdb-core/tests/pg_catalog_shims.rs`.

### Implementation for User Story 2

- [X] T010 [P] [US2] Add `ScopedRowKey` (`User(UserTableRowId)` | `Shared(SharedTableRowId)`) and `VersionedTableRow` with bit-identical `storekey` and KOBJ encodings in `backend/crates/kalamdb-commons/src/ids/row_id.rs` and `backend/crates/kalamdb-commons/src/models/rows/`.
- [X] T011 [US2] Replace `UserRowCodec`/`SharedRowCodec` and `UserTablePkIndex`/`SharedTablePkIndex` with `VersionedRowCodec` and `PrefixIndex<ScopedRowKey, VersionedTableRow>` under `backend/crates/kalamdb-tables/src/versioned/`, and delete `backend/crates/kalamdb-tables/src/user_tables/pk_index.rs` and `backend/crates/kalamdb-tables/src/shared_tables/pk_index.rs` (depends on T010).
- [ ] T012 [US2] Move the existing hot+cold MVCC scan, `count_resolved_rows_async`, PK lookup, ordinal decode, vector staging, `columnar_hot_scan`, `sum_hot_covering_integers`, `plan_literal_equality_updates`, and `TableMutationAccumulator` from `backend/crates/kalamdb-tables/src/user_tables/provider/{dml,storage}.rs` and `backend/crates/kalamdb-tables/src/shared_tables/provider/{dml,storage,columnar_scan,covering_aggregate,equality_update}.rs` into `backend/crates/kalamdb-tables/src/versioned/engine.rs`. Call those bodies. Do not rewrite them. This is the only index and hot/cold implementation for user, shared, stream, and persisted system rows (depends on T005, T011).
- [ ] T013 [US2] Keep stream-only TTL, expiry, and hard-delete in `backend/crates/kalamdb-tables/src/stream_tables/` as capability behavior. Delete any second index or hot/cold implementation in that directory so stream storage uses `VersionedTableEngine`. Allow explicit stream transactions through that engine (depends on T012).
- [X] T014 [US2] Implement `KalamTableProvider` and `TableCapabilityProfile` once in `backend/crates/kalamdb-tables/src/provider/` so `scan`, `insert_into`, `delete_from`, `update`, `supports_filters_pushdown`, and `statistics` call `VersionedTableEngine` for user, shared, and stream storage. Stream TTL and hard-delete stay capability checks. Shared tables pass `RlsMode::ForcePolicyCheck` into `SharedTableAuthorization` (depends on T012, T013).
- [X] T015 [US2] Store `Arc<KalamTableProvider>` on `CachedTableData` in `backend/crates/kalamdb-core/src/schema_registry/registry/core.rs` and delete `.downcast_ref::<UserTableProvider|SharedTableProvider|StreamTableProvider>()` from `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, `backend/crates/kalamdb-core/src/sql/executor/sql_executor/{mod,literal_dml,equality_update,covering_aggregate}.rs`, `backend/crates/kalamdb-core/src/sql/executor/transaction_batch_insert.rs`, `backend/crates/kalamdb-jobs/src/executors/{flush,user_export,table_transfer,stream_eviction}.rs`, `backend/crates/kalamdb-pg-bridge/src/service.rs`, `backend/crates/kalamdb-dba/src/repository/shared_repository.rs`, and `backend/crates/kalamdb-tables/src/shared_tables/shared_table_authorization.rs` (depends on T014).
- [ ] T016 [US2] Delete `UserTableProvider`, `SharedTableProvider`, `StreamTableProvider`, `BaseTableProvider`, `DeferredMvccScanProvider`, and `backend/crates/kalamdb-tables/src/utils/unified_dml/append.rs` after T015 callers compile. Keep `SharedTableAuthorization` in `backend/crates/kalamdb-tables/src/shared_tables/shared_table_authorization.rs` unchanged, and call it from `KalamTableProvider` for shared-table `RlsMode::ForcePolicyCheck` on read predicates and on insert/update `WITH CHECK` (depends on T015).
- [X] T017 [US2] Implement `KeyedTableProvider<K, V>` in `backend/crates/kalamdb-system/src/providers/keyed_provider.rs`: `check_system_table_access` inside `scan`, primary-key equality calls `store.get` then `build_record_batch!`, secondary predicates use the shared `PrefixIndex` iterator, and full scans use `build_record_batch!`. SQL `insert_into`/`update`/`delete_from` stay rejected. Direct `get`/`put`/`scan_index`/`update_with_old`/`delete` do not build a DataFusion plan or a `Row` round-trip. Do not copy index, hot/cold, or schema-history code into this file; call the shared implementation from T012 (depends on T006, T012).
- [ ] T018 [US2] Construct `KeyedTableProvider` from `Model::definition()` for users, jobs, job nodes, namespaces, storages, manifest, table policies, topics, topic offsets, migrations, audit logs, and the 11 `CatalogStores` tables, while leaving `get`/`put`/`scan_index` and the privileged-role cache on `IndexedEntityStore` in `backend/crates/kalamdb-system/src/providers/` (depends on T017).
- [ ] T019 [US2] Delete `impl_indexed_system_table_provider!`, `impl_simple_system_table_provider!`, `impl_system_table_provider_metadata!`, `SystemTableScan`, and `SimpleSystemTableScan` from `backend/crates/kalamdb-system/src/macros.rs` and `backend/crates/kalamdb-system/src/providers/base.rs`. Keep `build_record_batch!`, `array_builder!`, `append_value!`, and `finish_array!`. Delete `backend/crates/kalamdb-system/src/providers/catalog/scan.rs` and the 11 `*_provider.rs` files under that directory (`types_provider.rs`, `type_fields_provider.rs`, `routines_provider.rs`, `routine_parameters_provider.rs`, `routine_grants_provider.rs`, `function_modules_provider.rs`, `function_revisions_provider.rs`, `function_artifacts_provider.rs`, `schedules_provider.rs`, `triggers_provider.rs`, `trigger_attempts_provider.rs`). Keep `stores.rs`, `schedule_timing.rs`, and `catalog_tests.rs` (depends on T018).
- [X] T020 [P] [US2] Move virtual `system.*` row structs onto `#[table]` and delete the hand-built `ColumnDefinition` lists in `backend/crates/kalamdb-views/src/system/*.rs` and the helpers in `backend/crates/kalamdb-views/src/system/common.rs`. Keep each view's in-memory projection (depends on T006).
- [X] T021 [P] [US2] Move `pg_catalog` row structs onto `#[table(namespace = "pg_catalog")]` and delete hand-written `Field::new` `schema()` functions in `backend/crates/kalamdb-views/src/pg_catalog/*.rs`, starting with `class.rs`, `attribute.rs`, `namespace.rs`, `type.rs`, `proc.rs`, `constraint.rs`, and `index.rs`. Leave `compute_batch` and oid helpers unchanged as the reference projection. Do not rewrite those bodies (depends on T006).
- [ ] T022 [US2] Add `ComputedCatalogProvider` in `backend/crates/kalamdb-views/src/catalog_provider.rs` for access checks, projection, limit, and the macro schema. Register it from `backend/crates/kalamdb-views/src/pg_catalog/mod.rs`, `backend/crates/kalamdb-views/src/system/mod.rs`, `backend/crates/kalamdb-views/src/information_schema/extend.rs`, `backend/crates/kalamdb-core/src/views/system_schema_provider.rs`, and `backend/crates/kalamdb-core/src/app_context.rs`. Leave DataFusion's stock `InformationSchemaProvider` for relations Kalam does not own (depends on T020, T021).
- [X] T023 [US2] Move the existing user and shared flush bodies into one scope-aware `VersionedTableFlushJob` in `backend/crates/kalamdb-flush/src/flush/versioned.rs`. Do not rewrite the segment-encoding steps. Update `backend/crates/kalamdb-jobs/src/executors/flush.rs` and `backend/crates/kalamdb-flush/src/flush/mod.rs`, and delete `backend/crates/kalamdb-flush/src/flush/users.rs` and `backend/crates/kalamdb-flush/src/flush/shared.rs` only after the moved job compiles (depends on T012).
- [ ] T024 [US2] Re-run the T002 system-catalog workloads, write candidate median and p99 seconds beside the baseline in `specs/039-unified-table-transactions/baselines.md`, and stop if either regresses (depends on T016, T019, T022, T023).
- [ ] T063 [US2] Add failing tests that a shared table with `RlsMode::ForcePolicyCheck` still applies the current read predicate and insert/update `WITH CHECK` through `KalamTableProvider`, next to the existing policy tests in `backend/crates/kalamdb-tables/src/shared_tables/shared_table_authorization.rs` (depends on T016).
- [X] T064 [US2] Add the shared-view scan hook on `KalamTableProvider` in `backend/crates/kalamdb-tables/src/provider/` so a future SQL view over a shared table uses that provider and `SharedTableAuthorization`, with no second scan engine. Describe the hook in `specs/039-unified-table-transactions/contracts/table-processing-and-storage.md` (depends on T016).
- [ ] T065 [US2] Expose direct insert, update, and delete on `KalamTableProvider` in `backend/crates/kalamdb-tables/src/provider/` for typed rows, with no SQL parse. Keep `IndexedEntityStore::{get, put, scan_index, update_with_old, delete}` in `backend/crates/kalamdb-system/src/providers/` as the persisted-system API (depends on T014, T018).
- [ ] T066 [US2] Call the shared schema-history decoder for user, shared, stream, and persisted system reads from `backend/crates/kalamdb-tables/src/versioned/engine.rs`. In `backend/crates/kalamdb-system/src/providers/keyed_provider.rs`, use that decoder only when a historical layout is required, and still encode the current row with `build_record_batch!` (depends on T012, T017).

**Checkpoint**: User, shared, stream, persisted system, and computed catalog relations each have one provider for their capability. Story 2 is independently testable before atomic-commit work.

---

## Phase 4: User Story 1 - Atomic commit across unified table processing (Priority: P0)

**Goal**: One nonempty transaction in a supported domain becomes durable in one `StorageBackend::batch`, including stream rows and persisted-system direct-API mutations in that domain. A late validation failure or crash leaves nothing partial. Snapshot readers keep their frontier across hot replacement and flush. Shared-table RLS checks use the existing authorization code (FR-005–FR-012, SC-001, SC-002).

**Independent Test**: Identical 25,000-row insert, update, delete, and mixed-table cases on user and shared tables. Row 25,000 invalid leaves zero durable rows, vectors, file deletes, manifest changes, and notifications. A reader pinned to an older snapshot still sees the old hot row after a later flush.

### Tests for User Story 1

- [ ] T025 [P] [US1] Extend `backend/crates/kalamdb-core/tests/transaction_user_batch_commit.rs` and the shared-table transaction tests under `backend/crates/kalamdb-core/tests/` so a 25,000-row user commit and a 25,000-row shared commit each expect one `StorageBackend::batch` call, and a failure on the last row expects zero durable effects.
- [X] T026 [P] [US1] Add failing tests in `backend/crates/kalamdb-transactions/src/commit_sequence.rs` that `min_pinned_version(group_id)` tracks an active snapshot and releases it on commit, rollback, and timeout.

### Implementation for User Story 1

- [X] T027 [US1] Implement `min_pinned_version(group_id) -> Option<VersionId>` and RAII pin release on `CommitSequenceTracker` in `backend/crates/kalamdb-transactions/src/commit_sequence.rs` (depends on T026).
- [X] T028 [US1] Teach `IndexedEntityStore::push_replace` in `backend/crates/kalamdb-store/src/indexed_store.rs` to retain the prior hot row and primary-key index entry while `min_pinned_version` shows a reader with `old_version <= snapshot < new_version`, and to replace in place otherwise (depends on T027).
- [ ] T029 [US1] Replace the parallel user and shared apply, prevalidate, and cleanup methods in `backend/crates/kalamdb-core/src/applier/executor/dml.rs` with one prepare path that builds `PreparedCommitBatch` (`Vec<Operation>` plus `PostCommitEffectPlan`) through `VersionedTableEngine`, validating constraints, `FILE` refs, uniqueness, and byte/row/version limits before any storage write. Shared-table RLS `WITH CHECK` calls the existing `SharedTableAuthorization` from T016. Do not write a second policy checker (depends on T014, T016, T028).
- [ ] T030 [US1] Submit that batch with one `StorageBackend::batch` in `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, without `task_local!` coalescing across `spawn_blocking`, and run frontier advance, manifest cache update, vector staging, topic/live notification, and file cleanup only after `Ok` (depends on T029).
- [ ] T031 [US1] Reject cross-domain, incompatible-backend, and over-limit commits before any write in `backend/crates/kalamdb-transactions/src/engine.rs` and `backend/crates/kalamdb-core/src/applier/executor/dml.rs` (depends on T029).
- [ ] T032 [US1] Add a crash/replay test under `backend/crates/kalamdb-core/tests/` that an interrupted commit is wholly present or wholly absent and that replay does not duplicate logical effects (depends on T030).
- [ ] T067 [US1] Extend the one-batch commit in `backend/crates/kalamdb-core/src/applier/executor/dml.rs` so a transaction that includes stream rows and persisted-system direct-API mutations in one domain still submits one `StorageBackend::batch`. System mutations enter as store operations, not as a DataFusion plan (depends on T030, T013, T065).

**Checkpoint**: Uncoalesced atomic commit is proven for user and shared tables. Do not enable commit folding until T032 passes.

---

## Phase 5: User Story 3 - Less transaction work, same SQL results (Priority: P1)

**Goal**: Repeated changes to one key fold to the transition table. Dirty reads stay correct without converting the whole table to scalar rows (FR-013–FR-017, SC-003, SC-010).

**Independent Test**: All nine fold transitions, primary-key rename, savepoint rewind, explicit null, and both directions of predicate movement. 100-read and 1,000-read dirty transactions match unoptimized results. 25,000 operations on one key write storage bounded by distinct targets.

### Tests for User Story 3

- [ ] T033 [P] [US3] Add failing tests for the nine fold transitions, primary-key rename, savepoint rewind, and explicit null in `backend/crates/kalamdb-transactions/src/write_set.rs` tests, covering user and shared scopes.
- [ ] T034 [P] [US3] Add failing 100-read and 1,000-read dirty-transaction tests, including predicate movement and partial updates, in `backend/crates/kalamdb-transactions/src/overlay_exec.rs` tests.

### Implementation for User Story 3

- [ ] T035 [US3] Implement the nine-transition fold plus primary-key rename expansion keyed by `(TableId, TableScope, PkBucketKey)` in `backend/crates/kalamdb-transactions/src/write_set.rs`, preserving `ordered_mutations` for savepoint rollback and statement affected counts (depends on T030, T033).
- [ ] T036 [US3] Load each distinct `ScopedTargetKey` at most once per commit and check `expected_max_version` in `backend/crates/kalamdb-tables/src/versioned/target.rs` (depends on T035).
- [ ] T037 [US3] Push the minimal column projection from `KalamTableProvider::scan` in `backend/crates/kalamdb-tables/src/provider/` into `TransactionOverlayExec` in `backend/crates/kalamdb-transactions/src/overlay_exec.rs` so untouched batches stay columnar and are not converted to `Vec<Row>` (depends on T014, T034).
- [ ] T038 [US3] Record five matched `dev`-profile timings for the one-key fold and the 100/1,000 dirty reads in `specs/039-unified-table-transactions/baselines.md` (depends on T036, T037).

**Checkpoint**: Folding is on only after uncoalesced atomic commit passed. SQL results match the unfold path.

---

## Phase 6: User Story 4 - Deletion, history, and live synchronization (Priority: P1)

**Goal**: Ordinary secondary indexes omit new tombstones. Primary-key lookup still sees the latest tombstone. Deleted-row and history queries use a complete path. Complete-event replay stays distinct from latest-state reads. Live-query notifications cover user, shared, and stream commits, and the same helper runs after a persisted-system direct-API write (FR-018–FR-021, SC-004).

**Independent Test**: Primary-key, indexed, deleted-row, and history queries match authoritative results for hot, cold, and mixed rows. An expired live cursor reports a gap. An online index rebuild publishes one generation and recovers after failure.

### Tests for User Story 4

- [ ] T039 [P] [US4] Add failing tests that `IndexRole::PrimaryKey` retains the latest tombstone, `IndexRole::SecondaryScalar` omits a new tombstone, and `_deleted = true` / `since_seq` scans use the full authoritative path, in `backend/crates/kalamdb-store/src/indexed_store.rs` tests and `backend/crates/kalamdb-tables/src/versioned/engine.rs` tests.

### Implementation for User Story 4

- [ ] T040 [US4] Add `IndexRole::{PrimaryKey, SecondaryScalar}` on `PrefixIndex` in `backend/crates/kalamdb-store/src/index/prefix.rs` and apply it when `IndexedEntityStore` prepares operations in `backend/crates/kalamdb-store/src/indexed_store.rs` (depends on T039).
- [ ] T041 [US4] Route `_deleted = true` and `since_seq` queries through the complete hot-plus-cold scan in `backend/crates/kalamdb-tables/src/versioned/engine.rs`, bypassing `IndexRole::SecondaryScalar` (depends on T012, T040).
- [ ] T042 [US4] Keep complete-event delivery distinct from latest-state-since-cursor reads, and return an explicit gap when history is expired, in `backend/crates/kalamdb-core/src/live_adapters.rs`.
- [ ] T068 [US4] Send live-query notifications for user, shared, and stream commits from `backend/crates/kalamdb-core/src/live_adapters.rs`, and call that same helper after a persisted-system direct-API write in `backend/crates/kalamdb-system/src/providers/` (depends on T030, T042, T065).
- [ ] T043 [US4] Publish rebuilt index generations atomically and recover after a failed rebuild in `backend/crates/kalamdb-flush/src/compaction/small_segment.rs`, without dropping snapshot retention (depends on T027, T040).

**Checkpoint**: Delete and history results match the pre-change authoritative path. Secondary indexes are smaller only where the role allows it.

---

## Phase 7: User Story 5 - Responsive background work (Priority: P1)

**Goal**: Ordinary writes do not wait on bulk Parquet I/O. Transfers stream within a memory budget. Idle flush-scope locks are reclaimed (FR-022–FR-025, SC-006, SC-007).

**Independent Test**: With cold storage delayed by one second, `mark_pending_write` does not wait on that lock. Flushing table A does not invalidate unrelated warm entries for table B. A large export streams to disk on `spawn_blocking`.

### Tests for User Story 5

- [ ] T044 [P] [US5] Extend the `mark_pending_write` tests in `backend/crates/kalamdb-flush/src/service.rs` so a delayed one-second cold flush does not block an ordinary pending-write registration, and idle `flush_scope_locks` entries return to the steady-state bound after scope churn.

### Implementation for User Story 5

- [ ] T045 [US5] Decouple `mark_pending_write` from the bulk flush lock in `backend/crates/kalamdb-flush/src/service.rs`, and move Parquet write, rename, and scope hooks outside `flush_scope_lock` in `backend/crates/kalamdb-flush/src/flush/scope_writer.rs` (depends on T023, T044).
- [ ] T046 [US5] Reclaim idle `flush_scope_locks` entries in `backend/crates/kalamdb-flush/src/service.rs` so inactive scopes do not accumulate (depends on T045).
- [ ] T047 [US5] Stream ZIP backup, restore, table export, and user export to disk on `spawn_blocking` with a bounded buffer in `backend/crates/kalamdb-transfer/src/` and `backend/crates/kalamdb-jobs/src/executors/{table_transfer,user_export}.rs` (depends on T004).
- [ ] T048 [US5] Run the quickstart Gate 5 flush and transfer checks and record each runtime in seconds in `specs/039-unified-table-transactions/baselines.md` (depends on T046, T047).

**Checkpoint**: Pending writes proceed during a stalled cold flush. Archive jobs do not hold the request runtime for the whole file.

---

## Phase 8: User Story 6 - Same SQL results on every transport (Priority: P1)

**Goal**: Plan cache identity includes PostgreSQL star visibility. Concurrent misses share one plan. `RETURNING` and stream-target extraction use the AST (FR-026–FR-029, SC-008).

**Independent Test**: HTTP SQL and PostgreSQL wire `SELECT *` stay isolated in both warmup orders. Equivalent concurrent misses plan once. Quoted identifiers, comments, newlines, and nontrivial `RETURNING` describe the same columns as execution.

### Tests for User Story 6

- [ ] T049 [P] [US6] Add failing tests for both warmup orders of HTTP SQL versus wire `SELECT *`, and for a cancelled singleflight leader releasing waiters, in `backend/crates/kalamdb-plan-cache/src/lib.rs` tests and `backend/crates/kalamdb-core/src/sql/executor/sql_executor/planning.rs` tests.

### Implementation for User Story 6

- [ ] T050 [US6] Add `postgres_star` to `PlanCacheKey` in `backend/crates/kalamdb-plan-cache/src/lib.rs` and set it from `ExecutionContext::with_postgres_star` in `backend/crates/kalamdb-core/src/sql/context/execution_context.rs` and `backend/crates/kalamdb-postgres-wire/src/sql_exec.rs` (depends on T049).
- [ ] T051 [US6] Add `LeaderGuard` plus `Notify` singleflight around cold plan misses in `backend/crates/kalamdb-plan-cache/src/lib.rs`, with bounded waiters and release on failure or cancellation (depends on T050).
- [ ] T052 [US6] Replace the `returning_describe_sql` string round-trip in `backend/crates/kalamdb-dialect/src/parser/dml/insert_shape.rs` and the manual `extract_select_from_table_id` scan in `backend/crates/kalamdb-core/src/sql/executor/sql_executor/transactions.rs` with AST operations on `Statement` from `KalamParser`.

**Checkpoint**: Cache hits cannot cross the `postgres_star` boundary. Description and execution use one parse.

---

## Phase 9: User Story 7 - Safe upgrade and typed ownership (Priority: P2)

**Goal**: Row V2 writing turns on only after every eligible replica can read it. Job startup uses a typed capability. The stable-primary-key secondary-index redesign stays a separate gated migration (FR-030–FR-034, SC-011).

**Independent Test**: Mixed V1/V2 rows remain readable after restart. Enabling the V2 writer with an incompatible eligible replica is rejected. Disabling the writer is safe. Startup fails closed when the typed job capability is missing.

### Tests for User Story 7

- [ ] T053 [P] [US7] Add failing tests that mixed row formats stay readable, an incompatible eligible replica blocks `set_row_v2_writer_enabled(true)`, and writer disable leaves existing rows readable, in `backend/crates/kalamdb-serialization/src/object/envelope.rs` tests.

### Implementation for User Story 7

- [ ] T054 [US7] Gate `set_row_v2_writer_enabled` in `backend/crates/kalamdb-serialization/src/object/envelope.rs` on a durable cluster compatibility floor for every replica eligible to apply the new format (depends on T053).
- [ ] T055 [US7] Replace `OnceCell<Arc<dyn Any + Send + Sync>>` job manager access in `backend/crates/kalamdb-core/src/app_context.rs` with a typed job capability, and fail startup in `backend/crates/kalamdb-jobs/src/lib.rs` when that capability is not registered.
- [ ] T056 [US7] Leave the stable-primary-key secondary-index redesign unimplemented. Record the separate migration gate in `docs/architecture/decisions/` so it cannot ship inside the Row V2 flag (depends on T054).

**Checkpoint**: Format rollout and index redesign are independent switches. Ordinary job access does not downcast `Any`.

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: Prove the consolidation gates and update the docs the change actually affects.

- [ ] T057 Recompute production lines for the roots in `specs/039-unified-table-transactions/quickstart.md` and write removed, added, and net lines in `specs/039-unified-table-transactions/baselines.md`. Net reduction must be positive. File moves and deleted tests do not count (depends on T016, T019, T023).
- [ ] T058 Confirm `rg 'downcast_ref::<(User|Shared|Stream)TableProvider>' backend/crates/` returns no production hits, and confirm `backend/crates/kalamdb-transfer/Cargo.toml` does not depend on `kalamdb-core` and `backend/crates/kalamdb-publisher/Cargo.toml` does not depend on `kalamdb-tables` (depends on T003, T004, T015).
- [ ] T059 Update the transaction, storage, and crate-boundary pages under `docs/architecture/` to describe `KalamTableProvider`, `KeyedTableProvider`, `ComputedCatalogProvider`, one versioned flush job, and the system-catalog fast path (depends on T024, T030).
- [ ] T060 If a public SQL syntax, system table, CLI flag, config key, or SDK entry point changed, update the matching page under `../kalamdb-skills` and `../KalamSite/content/sdk/`. Skip this task when the diff is internal only.
- [ ] T061 Run the Gate 1–7 commands in `specs/039-unified-table-transactions/quickstart.md` with `cargo nextest run` on the default `dev` profile. CLI smoke (`cd cli && KALAMDB_SERVER_URL=http://localhost:3000 KALAMDB_ROOT_PASSWORD=mypass cargo nextest run -p kalam-cli-e2e --test e2e smoke`) runs only after a server is up (depends on T032, T038, T043, T048, T052, T055).
- [ ] T062 Review the touched SQL and scan paths in `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, `backend/crates/kalamdb-system/src/providers/keyed_provider.rs`, and `backend/crates/kalamdb-views/src/catalog_provider.rs` for SQL injection, system-table auth bypass, and non-admin access to `system.*` through views or unions (depends on T022, T030).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: Start immediately. T002 waits on T001.
- **Foundational (Phase 2)**: Starts after T002. Blocks every user story.
- **User Story 2 (Phase 3)**: Starts after Phase 2. Blocks Story 1's engine tasks (`T029` needs `T014` and `T016`). `T063`–`T066` finish inside Story 2.
- **User Story 1 (Phase 4)**: Starts after `T014` and `T027`. Folding (`T035`) waits until `T032` passes. `T067` waits on `T030`, `T013`, and `T065`.
- **User Stories 3–6**: Start after their named dependencies. Stories 4, 5, and 6 can proceed in parallel once Story 2's engine and flush job exist.
- **User Story 7**: Starts after Phase 2. It does not require Stories 3–6.
- **Polish**: Starts after the stories being shipped. `T061` waits on the P0 and P1 gates that are in scope.

### User Story Dependencies

- **User Story 2 (P0)**: After Phase 2. No dependency on other stories.
- **User Story 1 (P0)**: After `T014` from Story 2, plus its own snapshot-pin tasks. This is the spec's required order: shared processing first, then uncoalesced atomic commit.
- **User Story 3 (P1)**: After `T030`. Folding is invalid before uncoalesced atomic commit is proven.
- **User Story 4 (P1)**: After `T012`. Snapshot retention in `T043` also uses `T027`. Live query for system writes (`T068`) also waits on `T065`.
- **User Story 5 (P1)**: After `T023` and `T004`.
- **User Story 6 (P1)**: After Phase 2. Independent of the storage engine.
- **User Story 7 (P2)**: After Phase 2. Independent of Stories 3–6.

### Within Each User Story

- Tests are written to fail before the implementation task that names them as a dependency.
- Models and keys before engines.
- Engines before the DataFusion provider.
- Provider before caller downcast removal.
- Downcast removal before deleting the old provider files.
- Story checkpoint before the next priority that depends on it.

### Parallel Opportunities

- `T003`, `T004`, `T005`, and `T006` touch different crates and can run together.
- `T007`, `T008`, and `T009` can run together.
- `T010`, `T020`, and `T021` can run together after Phase 2. `T013` waits on `T012`. `T017` waits on `T012`.
- `T025` and `T026` can run together.
- `T033` and `T034` can run together.
- After `T014` and `T023`, Story 4, Story 5, and Story 6 can run together.
- Story 7 can run beside Stories 4–6.

---

## Parallel Example: User Story 2

```text
# After Phase 2, start together:
T007  parity tests in kalamdb-tables/tests and kalamdb-core sql executor tests
T008  direct-store lookup tests in kalamdb-system providers
T009  pg_catalog schema tests in kalamdb-views and kalamdb-core
T010  ScopedRowKey and VersionedTableRow in kalamdb-commons
T020  #[table] on virtual system views in kalamdb-views/src/system
T021  #[table] on pg_catalog rows in kalamdb-views/src/pg_catalog

# Then, still in parallel once their inputs exist:
T012  move existing engine bodies into VersionedTableEngine (after T010, T011)
T013  stream capabilities on that engine                    (after T012)
T017  KeyedTableProvider calling the shared index          (after T012)
T018  wire persisted system stores                          (after T017)
T022  ComputedCatalogProvider                               (after T020, T021)
T023  move existing flush bodies into VersionedTableFlushJob (after T012)
T063  shared RLS regression tests                           (after T016)
T064  shared-view scan hook                                 (after T016)
T065  direct non-SQL mutation API                           (after T014, T018)
T066  shared schema-history decoder                         (after T012, T017)
```

---

## Implementation Strategy

### MVP (Stories 2 and 1)

1. Finish Phase 1 and Phase 2.
2. Finish Phase 3 (Story 2) and pass `T024` (SC-015).
3. Finish Phase 4 (Story 1) and pass `T032`.
4. Stop. User and shared commits are atomic on one provider, and system catalog latency has not regressed.

Story 1 alone is not the MVP. Its commit path is specified on the unified engine from Story 2.

### Incremental Delivery

1. MVP above.
2. Story 3 folding and columnar overlay, after uncoalesced commit is proven.
3. Stories 4, 5, and 6 in parallel.
4. Story 7 format gate and typed jobs.
5. Polish gates in `quickstart.md`.

### Parallel Team Strategy

1. One person completes Phase 1 and Phase 2.
2. Then split Story 2: one person on `kalamdb-tables` (`T010`–`T016`, `T023`, `T063`, `T064`, `T065`), one person on `kalamdb-system` (`T017`–`T019`, `T066`), one person on `kalamdb-views` (`T020`–`T022`). The views work keeps `compute_batch` unchanged.
3. After `T014`, one person takes Story 1 while the others finish catalog wiring and the flush job.
4. After `T032`, split Stories 4, 5, and 6.

---

## Notes

- `[P]` tasks use different files and do not wait on an unfinished task.
- System catalog `get`/`put`/`scan_index`/`update_with_old`/`delete` stay off DataFusion. They call the shared index and schema-history code. They do not grow a second copy of it.
- `#[table]` and `build_record_batch!` stay. Provider-impl macros and hand-written duplicate schemas go.
- `pg_catalog` `compute_batch` bodies, `SharedTableAuthorization`, and the hot-scan, covering-aggregate, and equality-update bodies stay the reference. Move or call them. Do not rewrite them.
- Shared tables keep forced row-level security. A later view over a shared table uses `KalamTableProvider` and that same security.
- Commit after each task or a coherent group. Do not start Story 3 folding until Story 1's uncoalesced commit tests pass.
