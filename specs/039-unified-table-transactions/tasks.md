# Tasks: Unified Table Processing and Atomic Transaction Optimization

**Input**: Design documents from `/specs/039-unified-table-transactions/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md, [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md)

The performance program (T069–T089) implements that handoff exactly. Each task's handoff section is the requirement, including its acceptance text. Architecture acceptance and correctness gates 1–8 in the handoff apply to every performance task. The copy in this folder is the same text as `docs/plans/2026-10-30-dml-storage-performance-handoff.md`.

**Tests**: Included. FR-035, the story independent tests, and `quickstart.md` require them. Write each story's tests so they fail before the implementation task they guard. Use the default `dev` profile. Report runtimes in seconds.

**Branch**: Stay on `feat/pg-dialect` (PR #402). Do not open a new branch.

**Organization**: Tasks follow user stories. Story 2 is sequenced before Story 1 because the atomic commit in Story 1 is implemented on `VersionedTableEngine` from Story 2. Both are P0. The MVP is Story 2 plus Story 1.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on an incomplete task)
- **[Story]**: User story label (`[US1]` … `[US7]`)
- Setup, Foundational, Polish, and performance-handoff tasks have no story label. Performance tasks are named by their handoff id (`A1` … `F3`).

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
- [X] T012 [US2] Move the existing hot+cold MVCC scan, `count_resolved_rows_async`, PK lookup, ordinal decode, vector staging, `columnar_hot_scan`, `sum_hot_covering_integers`, `plan_literal_equality_updates`, and `TableMutationAccumulator` from `backend/crates/kalamdb-tables/src/user_tables/provider/{dml,storage}.rs` and `backend/crates/kalamdb-tables/src/shared_tables/provider/{dml,storage,columnar_scan,covering_aggregate,equality_update}.rs` into `backend/crates/kalamdb-tables/src/versioned/engine.rs`. Call those bodies. Do not rewrite them. This is the only index and hot/cold implementation for user, shared, stream, and persisted system rows (depends on T005, T011).
- [X] T013 [US2] Keep stream-only TTL, expiry, and hard-delete in `backend/crates/kalamdb-tables/src/stream_tables/` on `StreamTableEngine` and `StreamTableStore`. Do not add a second index or hot/cold path, and do not move stream storage onto `VersionedTableEngine`. Explicit stream transactions stay rejected (depends on T012).
- [X] T014 [US2] Implement `KalamTableProvider` and `TableCapabilityProfile` once in `backend/crates/kalamdb-tables/src/provider/` so `scan`, `insert_into`, `delete_from`, `update`, `supports_filters_pushdown`, and `statistics` call `VersionedTableEngine` for user, shared, and stream storage. Stream TTL and hard-delete stay capability checks. Shared tables pass `RlsMode::ForcePolicyCheck` into `SharedTableAuthorization` (depends on T012, T013).
- [X] T015 [US2] Store `Arc<KalamTableProvider>` on `CachedTableData` in `backend/crates/kalamdb-core/src/schema_registry/registry/core.rs` and delete `.downcast_ref::<UserTableProvider|SharedTableProvider|StreamTableProvider>()` from `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, `backend/crates/kalamdb-core/src/sql/executor/sql_executor/{mod,literal_dml,equality_update,covering_aggregate}.rs`, `backend/crates/kalamdb-core/src/sql/executor/transaction_batch_insert.rs`, `backend/crates/kalamdb-jobs/src/executors/{flush,user_export,table_transfer,stream_eviction}.rs`, `backend/crates/kalamdb-pg-bridge/src/service.rs`, `backend/crates/kalamdb-dba/src/repository/shared_repository.rs`, and `backend/crates/kalamdb-tables/src/shared_tables/shared_table_authorization.rs` (depends on T014).
- [X] T016 [US2] Keep `UserTableProvider`, `SharedTableProvider`, `BaseTableProvider`, and `DeferredMvccScanProvider` as the private scan and DML implementation behind `KalamTableProvider`. They are `pub(crate)`. External crates call `KalamTableProvider`. `VersionedTableEngine` stays crate-private. `StreamTableEngine` stays public. `backend/crates/kalamdb-tables/src/utils/unified_dml/append.rs` is deleted. `SharedTableAuthorization` stays and is called for shared `RlsMode::ForcePolicyCheck` on read predicates and on insert/update `WITH CHECK` (depends on T015).
- [X] T017 [US2] Implement `KeyedTableProvider<K, V>` in `backend/crates/kalamdb-system/src/providers/keyed_provider.rs`: `check_system_table_access` inside `scan`, primary-key equality calls `store.get` then `build_record_batch!`, secondary predicates use the shared `PrefixIndex` iterator, and full scans use `build_record_batch!`. SQL `insert_into`/`update`/`delete_from` stay rejected. Direct `get`/`put`/`scan_index`/`update_with_old`/`delete` do not build a DataFusion plan or a `Row` round-trip. Do not copy index, hot/cold, or schema-history code into this file; call the shared implementation from T012 (depends on T006, T012).
- [X] T018 [US2] Construct `KeyedTableProvider` from `Model::definition()` for users, jobs, job nodes, namespaces, storages, manifest, table policies, topics, topic offsets, migrations, audit logs, and the 11 `CatalogStores` tables, while leaving `get`/`put`/`scan_index` and the privileged-role cache on `IndexedEntityStore` in `backend/crates/kalamdb-system/src/providers/` (depends on T017).
- [X] T019 [US2] Delete `impl_indexed_system_table_provider!`, `impl_simple_system_table_provider!`, `impl_system_table_provider_metadata!`, `SystemTableScan`, and `SimpleSystemTableScan` from `backend/crates/kalamdb-system/src/macros.rs` and `backend/crates/kalamdb-system/src/providers/base.rs`. Keep `build_record_batch!`, `array_builder!`, `append_value!`, and `finish_array!`. Delete `backend/crates/kalamdb-system/src/providers/catalog/scan.rs` and the 11 `*_provider.rs` files under that directory (`types_provider.rs`, `type_fields_provider.rs`, `routines_provider.rs`, `routine_parameters_provider.rs`, `routine_grants_provider.rs`, `function_modules_provider.rs`, `function_revisions_provider.rs`, `function_artifacts_provider.rs`, `schedules_provider.rs`, `triggers_provider.rs`, `trigger_attempts_provider.rs`). Keep `stores.rs`, `schedule_timing.rs`, and `catalog_tests.rs` (depends on T018).
- [X] T020 [P] [US2] Move virtual `system.*` row structs onto `#[table]` and delete the hand-built `ColumnDefinition` lists in `backend/crates/kalamdb-views/src/system/*.rs` and the helpers in `backend/crates/kalamdb-views/src/system/common.rs`. Keep each view's in-memory projection (depends on T006).
- [X] T021 [P] [US2] Move `pg_catalog` row structs onto `#[table(namespace = "pg_catalog")]` and delete hand-written `Field::new` `schema()` functions in `backend/crates/kalamdb-views/src/pg_catalog/*.rs`, starting with `class.rs`, `attribute.rs`, `namespace.rs`, `type.rs`, `proc.rs`, `constraint.rs`, and `index.rs`. Leave `compute_batch` and oid helpers unchanged as the reference projection. Do not rewrite those bodies (depends on T006).
- [X] T022 [US2] Add `ComputedCatalogProvider` in `backend/crates/kalamdb-views/src/catalog_provider.rs` for access checks, projection, limit, and the macro schema. Register it from `backend/crates/kalamdb-views/src/pg_catalog/mod.rs`, `backend/crates/kalamdb-views/src/system/mod.rs`, `backend/crates/kalamdb-views/src/information_schema/extend.rs`, `backend/crates/kalamdb-core/src/views/system_schema_provider.rs`, and `backend/crates/kalamdb-core/src/app_context.rs`. Leave DataFusion's stock `InformationSchemaProvider` for relations Kalam does not own (depends on T020, T021).
- [X] T023 [US2] Move the existing user and shared flush bodies into one scope-aware `VersionedTableFlushJob` in `backend/crates/kalamdb-flush/src/flush/versioned.rs`. Do not rewrite the segment-encoding steps. Update `backend/crates/kalamdb-jobs/src/executors/flush.rs` and `backend/crates/kalamdb-flush/src/flush/mod.rs`, and delete `backend/crates/kalamdb-flush/src/flush/users.rs` and `backend/crates/kalamdb-flush/src/flush/shared.rs` only after the moved job compiles (depends on T012).
- [X] T024 [US2] Re-run the T002 system-catalog workloads, write candidate median and p99 seconds beside the baseline in `specs/039-unified-table-transactions/baselines.md`, and stop if either regresses (depends on T016, T019, T022, T023).
- [X] T063 [US2] Add failing tests that a shared table with `RlsMode::ForcePolicyCheck` still applies the current read predicate and insert/update `WITH CHECK` through `KalamTableProvider`, next to the existing policy tests in `backend/crates/kalamdb-tables/src/shared_tables/shared_table_authorization.rs` (depends on T016).
- [X] T064 [US2] Add the shared-view scan hook on `KalamTableProvider` in `backend/crates/kalamdb-tables/src/provider/` so a future SQL view over a shared table uses that provider and `SharedTableAuthorization`, with no second scan engine. Describe the hook in `specs/039-unified-table-transactions/contracts/table-processing-and-storage.md` (depends on T016).
- [X] T065 [US2] Expose direct insert, update, and delete on `KalamTableProvider` in `backend/crates/kalamdb-tables/src/provider/` for typed rows, with no SQL parse. Keep `IndexedEntityStore::{get, put, scan_index, update_with_old, delete}` in `backend/crates/kalamdb-system/src/providers/` as the persisted-system API (depends on T014, T018).
- [X] T066 [US2] User and shared historical reads decode through `VersionedTableEngine::decode_row`, which uses `StorageSchema::layout_for_version`. Persisted system current rows stay on `current_layout_batch` and `build_record_batch!` in `backend/crates/kalamdb-system/src/providers/keyed_provider.rs`. Stream rows are not `ScopedRowKey` KOBJ and stay on `StreamTableEngine`. `kalamdb-system` does not depend on `kalamdb-tables` (depends on T012, T017).

**Checkpoint**: User, shared, stream, persisted system, and computed catalog relations each have one provider for their capability. Story 2 is independently testable before atomic-commit work.

---

## Phase 4: User Story 1 - Atomic commit across unified table processing (Priority: P0)

**Goal**: One nonempty user or shared transaction in one compatible commit domain becomes durable in one `StorageBackend::batch`. Explicit stream transactions and persisted-system transactions stay rejected. A late validation failure or crash leaves nothing partial. Snapshot readers keep their frontier across hot replacement and flush. Shared-table RLS checks use the existing authorization code (FR-005–FR-012, SC-001, SC-002).

**Independent Test**: Identical 25,000-row insert, update, delete, and mixed-table cases on user and shared tables. Row 25,000 invalid leaves zero durable rows, vectors, file deletes, manifest changes, and notifications. A reader pinned to an older snapshot still sees the old hot row after a later flush.

### Tests for User Story 1

- [X] T025 [P] [US1] Extend `backend/crates/kalamdb-core/tests/transaction_user_batch_commit.rs` and the shared-table transaction tests under `backend/crates/kalamdb-core/tests/` so a 25,000-row user commit and a 25,000-row shared commit each expect one `StorageBackend::batch` call, and a failure on the last row expects zero durable effects.
- [X] T026 [P] [US1] Add failing tests in `backend/crates/kalamdb-transactions/src/commit_sequence.rs` that `min_pinned_version(group_id)` tracks an active snapshot and releases it on commit, rollback, and timeout.

### Implementation for User Story 1

- [X] T027 [US1] Implement `min_pinned_version(group_id) -> Option<VersionId>` and RAII pin release on `CommitSequenceTracker` in `backend/crates/kalamdb-transactions/src/commit_sequence.rs` (depends on T026).
- [X] T028 [US1] Teach `IndexedEntityStore::push_replace` in `backend/crates/kalamdb-store/src/indexed_store.rs` to retain the prior hot row and primary-key index entry while `min_pinned_version` shows a reader with `old_version <= snapshot < new_version`, and to replace in place otherwise (depends on T027).
- [X] T029 [US1] Replace the parallel user and shared apply paths in `backend/crates/kalamdb-core/src/applier/executor/dml.rs` with one prepare path that builds `PreparedCommitBatch` (`Vec<Operation>` plus `PostCommitEffectPlan`) through `KalamTableProvider` plan methods. The private versioned engine stays inside `kalamdb-tables`. Validate constraints, `FILE` refs, uniqueness, and byte/row/version limits before any storage write. Shared RLS `WITH CHECK` stays on the statement-stage `SharedTableAuthorization` and is not repeated at apply. Do not write a second policy checker (depends on T014, T016, T028).
- [X] T030 [US1] Submit that batch with one `StorageBackend::batch` in `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, without `task_local!` coalescing across `spawn_blocking`, and run frontier advance, manifest cache update, vector staging, topic/live notification, and file cleanup only after `Ok` (depends on T029).
- [X] T031 [US1] Reject cross-domain, incompatible-backend, and over-limit commits before any write in `backend/crates/kalamdb-transactions/src/engine.rs` and `backend/crates/kalamdb-core/src/applier/executor/dml.rs` (depends on T029).
- [X] T032 [US1] Add a crash/replay test under `backend/crates/kalamdb-core/tests/` that an interrupted commit is wholly present or wholly absent and that replay does not duplicate logical effects (depends on T030).
- [X] T067 [US1] Keep explicit transactions on stream tables and persisted system tables rejected (`stream tables are not supported inside explicit transactions`, `system tables are not supported inside explicit transactions`). A mixed commit may include only user and shared mutations that resolve to one compatible data domain, and that commit still submits one `StorageBackend::batch`. Do not submit stream rows or system mutations in that batch (FR-004, FR-008; depends on T030).

**Checkpoint**: Uncoalesced atomic commit is proven for user and shared tables. T032 passed. Commit folding stays disabled.

---

## Phase 5: User Story 3 - Less transaction work, same SQL results (Priority: P1)

**Goal**: Repeated changes to one key fold to the transition table. Dirty reads stay correct without converting the whole table to scalar rows (FR-013–FR-017, SC-003, SC-010).

**Independent Test**: All nine fold transitions, primary-key rename, savepoint rewind, explicit null, and both directions of predicate movement. 100-read and 1,000-read dirty transactions match unoptimized results. 25,000 operations on one key write storage bounded by distinct targets.

### Tests for User Story 3

- [X] T033 [P] [US3] Add failing tests for the nine fold transitions, primary-key rename, savepoint rewind, and explicit null in `backend/crates/kalamdb-transactions/src/write_set.rs` tests, covering user and shared scopes.
- [X] T034 [P] [US3] Add failing 100-read and 1,000-read dirty-transaction tests, including predicate movement and partial updates, in `backend/crates/kalamdb-transactions/src/overlay_exec/` tests.

### Implementation for User Story 3

- [X] T035 [US3] Implement the nine-transition fold plus primary-key rename expansion keyed by `(TableId, TableScope, PkBucketKey)` in `backend/crates/kalamdb-transactions/src/write_set.rs`, preserving `ordered_mutations` for savepoint rollback and statement affected counts (depends on T030, T033).
- [X] T036 [US3] Load each distinct `ScopedTargetKey` at most once per commit and check `expected_max_version` in `backend/crates/kalamdb-tables/src/versioned/target.rs` (depends on T035).
- [X] T037 [US3] Push the minimal column projection from `KalamTableProvider::scan` in `backend/crates/kalamdb-tables/src/provider/` into `TransactionOverlayExec` in `backend/crates/kalamdb-transactions/src/overlay_exec.rs` so untouched batches stay columnar and are not converted to `Vec<Row>` (depends on T014, T034).
- [X] T038 [US3] Record five matched `dev`-profile timings for the one-key fold and the 100/1,000 dirty reads in `specs/039-unified-table-transactions/baselines.md` (depends on T036, T037).

**Checkpoint**: Folding is on. Uncoalesced atomic commit already passed. SQL results match the unfold path: statement counts stay on the ordered mutation log, and storage receives one folded mutation per scoped key.

---

## Phase 6: User Story 4 - Deletion, history, and live synchronization (Priority: P1)

**Goal**: Ordinary secondary indexes omit new tombstones. Primary-key lookup still sees the latest tombstone. Deleted-row and history queries use a complete path. Complete-event replay stays distinct from latest-state reads. Live-query notifications cover user, shared, and stream commits, and the same helper runs after a persisted-system direct-API write (FR-018–FR-021, SC-004).

**Independent Test**: Primary-key, indexed, deleted-row, and history queries match authoritative results for hot, cold, and mixed rows. An expired live cursor reports a gap. An online index rebuild publishes one generation and recovers after failure.

### Tests for User Story 4

- [X] T039 [P] [US4] Add failing tests that `IndexRole::PrimaryKey` retains the latest tombstone, `IndexRole::SecondaryScalar` omits a new tombstone, and `_deleted = true` / `since_seq` scans use the full authoritative path, in `backend/crates/kalamdb-store/src/indexed_store.rs` tests and `backend/crates/kalamdb-tables/src/versioned/engine.rs` tests.

### Implementation for User Story 4

- [X] T040 [US4] Add `IndexRole::{PrimaryKey, SecondaryScalar}` on `PrefixIndex` in `backend/crates/kalamdb-store/src/index/prefix.rs` and apply it when `IndexedEntityStore` prepares operations in `backend/crates/kalamdb-store/src/indexed_store.rs` (depends on T039).
- [X] T041 [US4] Route `_deleted = true` and `since_seq` queries through the complete hot-plus-cold scan in `backend/crates/kalamdb-tables/src/versioned/engine.rs`, bypassing `IndexRole::SecondaryScalar` (depends on T012, T040).
- [X] T042 [US4] Keep complete-event delivery distinct from latest-state-since-cursor reads, and return an explicit gap when history is expired, in `backend/crates/kalamdb-core/src/live_adapters.rs`.
- [X] T068 [US4] Send live-query notifications for user, shared, and stream commits from `backend/crates/kalamdb-core/src/live_adapters.rs`, and call that same helper after a persisted-system direct-API write in `backend/crates/kalamdb-system/src/providers/` (depends on T030, T042, T065).
- [X] T043 [US4] Publish rebuilt index generations atomically and recover after a failed rebuild in `backend/crates/kalamdb-flush/src/compaction/small_segment.rs`, without dropping snapshot retention (depends on T027, T040).

**Checkpoint**: Delete and history results match the pre-change authoritative path. Secondary indexes are smaller only where the role allows it.

---

## Phase 7: User Story 5 - Responsive background work (Priority: P1)

**Goal**: Ordinary writes do not wait on bulk Parquet I/O. Transfers stream within a memory budget. Idle flush-scope locks are reclaimed (FR-022–FR-025, SC-006, SC-007).

**Independent Test**: With cold storage delayed by one second, `mark_pending_write` does not wait on that lock. Flushing table A does not invalidate unrelated warm entries for table B. A large export streams to disk on `spawn_blocking`.

### Tests for User Story 5

- [X] T044 [P] [US5] Extend the `mark_pending_write` tests in `backend/crates/kalamdb-flush/src/service.rs` so a delayed one-second cold flush does not block an ordinary pending-write registration, and idle `flush_scope_locks` entries return to the steady-state bound after scope churn.

### Implementation for User Story 5

- [X] T045 [US5] Decouple `mark_pending_write` from the bulk flush lock in `backend/crates/kalamdb-flush/src/service.rs`, and move Parquet write, rename, and scope hooks outside `flush_scope_lock` in `backend/crates/kalamdb-flush/src/flush/scope_writer.rs` (depends on T023, T044).
- [X] T046 [US5] Reclaim idle `flush_scope_locks` entries in `backend/crates/kalamdb-flush/src/service.rs` so inactive scopes do not accumulate (depends on T045).
- [X] T047 [US5] Stream ZIP backup, restore, table export, and user export to disk on `spawn_blocking` with a bounded buffer in `backend/crates/kalamdb-transfer/src/` and `backend/crates/kalamdb-jobs/src/executors/{table_transfer,user_export}.rs` (depends on T004).
- [X] T048 [US5] Run the quickstart Gate 5 flush and transfer checks and record each runtime in seconds in `specs/039-unified-table-transactions/baselines.md` (depends on T046, T047).

**Checkpoint**: Pending writes proceed during a stalled cold flush. Archive jobs do not hold the request runtime for the whole file.

---

## Phase 8: User Story 6 - Same SQL results on every transport (Priority: P1)

**Goal**: Plan cache identity includes PostgreSQL star visibility. Concurrent misses share one plan. `RETURNING` and stream-target extraction use the AST (FR-026–FR-029, SC-008).

**Independent Test**: HTTP SQL and PostgreSQL wire `SELECT *` stay isolated in both warmup orders. Equivalent concurrent misses plan once. Quoted identifiers, comments, newlines, and nontrivial `RETURNING` describe the same columns as execution.

### Tests for User Story 6

- [X] T049 [P] [US6] Add failing tests for both warmup orders of HTTP SQL versus wire `SELECT *`, and for a cancelled singleflight leader releasing waiters, in `backend/crates/kalamdb-plan-cache/src/lib.rs` tests and `backend/crates/kalamdb-core/src/sql/executor/sql_executor/planning.rs` tests.

### Implementation for User Story 6

- [X] T050 [US6] Add `postgres_star` to `PlanCacheKey` in `backend/crates/kalamdb-plan-cache/src/lib.rs` and set it from `ExecutionContext::with_postgres_star` in `backend/crates/kalamdb-core/src/sql/context/execution_context.rs` and `backend/crates/kalamdb-postgres-wire/src/sql_exec.rs` (depends on T049).
- [X] T051 [US6] Add `LeaderGuard` plus `Notify` singleflight around cold plan misses in `backend/crates/kalamdb-plan-cache/src/lib.rs`, with bounded waiters and release on failure or cancellation (depends on T050). The concrete behavior is handoff F1, tasked as T085. Do not add a second coordinator. Checking off T085 checks off T051.
- [X] T052 [US6] Replace the `returning_describe_sql` string round-trip in `backend/crates/kalamdb-dialect/src/parser/dml/insert_shape.rs` and the manual `extract_select_from_table_id` scan in `backend/crates/kalamdb-core/src/sql/executor/sql_executor/transactions.rs` with AST operations on `Statement` from `KalamParser`.

**Checkpoint**: Cache hits cannot cross the `postgres_star` boundary. Description and execution use one parse.

---

## Phase 9: User Story 7 - Safe upgrade and typed ownership (Priority: P2)

**Goal**: Row V2 writing turns on only after every eligible replica can read it. Job startup uses a typed capability. The stable-primary-key secondary-index redesign stays a separate gated migration (FR-030–FR-034, SC-011).

**Independent Test**: Mixed V1/V2 rows remain readable after restart. Enabling the V2 writer with an incompatible eligible replica is rejected. Disabling the writer is safe. Startup fails closed when the typed job capability is missing.

### Tests for User Story 7

- [X] T053 [P] [US7] Add failing tests that mixed row formats stay readable, an incompatible eligible replica blocks `set_row_v2_writer_enabled(true)`, and writer disable leaves existing rows readable, in `backend/crates/kalamdb-serialization/src/object/envelope.rs` tests.

### Implementation for User Story 7

- [X] T054 [US7] Gate `set_row_v2_writer_enabled` in `backend/crates/kalamdb-serialization/src/object/envelope.rs` on a durable cluster compatibility floor for every replica eligible to apply the new format (depends on T053).
- [X] T055 [US7] Replace `OnceCell<Arc<dyn Any + Send + Sync>>` job manager access in `backend/crates/kalamdb-core/src/app_context.rs` with a typed job capability, and fail startup in `backend/crates/kalamdb-jobs/src/lib.rs` when that capability is not registered.
- [X] T056 [US7] Leave the stable-primary-key secondary-index redesign unimplemented. Record the separate migration gate in `docs/architecture/decisions/` so it cannot ship inside the Row V2 flag (depends on T054). Handoff E1 (T082) is that investigation: measure first, write the ADR, and do not migrate the format in this task.

**Checkpoint**: Format rollout and index redesign are independent switches. Ordinary job access does not downcast `Any`.

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: Prove the consolidation gates and update the docs the change actually affects.

- [X] T057 Recompute production lines for the roots in `specs/039-unified-table-transactions/quickstart.md` and write removed, added, and net lines in `specs/039-unified-table-transactions/baselines.md`. Net reduction must be positive. File moves and deleted tests do not count (depends on T016, T019, T023).
- [X] T058 Confirm `rg 'downcast_ref::<(User|Shared|Stream)TableProvider>' backend/crates/` returns no production hits, and confirm `backend/crates/kalamdb-transfer/Cargo.toml` does not depend on `kalamdb-core` and `backend/crates/kalamdb-publisher/Cargo.toml` does not depend on `kalamdb-tables` (depends on T003, T004, T015).
- [X] T059 Update the transaction, storage, and crate-boundary pages under `docs/architecture/` to describe `KalamTableProvider`, `KeyedTableProvider`, `ComputedCatalogProvider`, one versioned flush job, and the system-catalog fast path (depends on T024, T030).
- [X] T060 If a public SQL syntax, system table, CLI flag, config key, or SDK entry point changed, update the matching page under `../kalamdb-skills` and `../KalamSite/content/sdk/`. Skip this task when the diff is internal only.
- [X] T061 Run the Gate 1–7 commands in `specs/039-unified-table-transactions/quickstart.md` with `cargo nextest run` on the default `dev` profile. CLI smoke (`cd cli && KALAMDB_SERVER_URL=http://localhost:3000 KALAMDB_ROOT_PASSWORD=mypass cargo nextest run -p kalam-cli-e2e --test e2e smoke`) runs only after a server is up (depends on T032, T038, T043, T048, T052, T055).
- [X] T062 Review the touched SQL and scan paths in `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, `backend/crates/kalamdb-system/src/providers/keyed_provider.rs`, and `backend/crates/kalamdb-views/src/catalog_provider.rs` for SQL injection, system-table auth bypass, and non-admin access to `system.*` through views or unions (depends on T022, T030).

---

## Phase 11: Performance handoff A — evidence and small avoidable costs

**Source**: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) Phase A. Same text as `docs/plans/2026-10-30-dml-storage-performance-handoff.md`.

**Goal**: A reproducible current `dev`-profile baseline, stage attribution for #8, #11, #12, #13, and #15, and no key/value allocation on the inactive coalescing path.

**Rules**: Revalidate folding, `TargetResolutionCache`, `prepare_mixed_versioned_commit` / `submit_prepared_commit`, and overlay behavior before editing. Extend what is already implemented. Preserve the dirty tree. Do not commit. Do not build `--release` unless the user explicitly overrides the repository rule. Append one evidence row to `baselines.md` when the slice is done. Pass the handoff review gate: ownership/call-path map before the edit; shared entry point, remaining table-specific differences, deleted paths, production-code delta, and any new public interface after it.

- [X] T069 A1. Reproducible benchmark and attribution, exactly handoff §A1. Files: `benchv2/pgwire-speed/src/main.rs`, `benchv2/pgwire-speed/run.sh`, `benchv2/pgwire-speed/README.md`, plus existing observability hooks and only the DML/storage paths this instrumentation touches. Preserve saved results. Store new runs in distinct directories with source commit, dirty-diff fingerprint, binary fingerprint, toolchain/profile/features, SQL file hashes, server settings, and timestamps. The existing script hardcodes release; make a same-profile `dev` before/after baseline. Do not compare `dev` results to the October release timings. One discarded warm-up suite and at least five recorded complete suites per version. Reset data for each suite. Preserve the 16-script order and dependencies. Alternate engine run order. Document cache conditions. Do not time one dependent script against an arbitrary database state. Record median and range per script and total, in seconds. Settings: one connection, one simple-query message per script, WAL enabled, fsync-wait policy matched, hot-only flush. Keep server startup and auth outside SQL timings. Add low-overhead aggregate counters and timing spans for planning, scan/filter, overlay, staging/folding, target resolution, row encoding/index construction, backend submission, and post-commit work. No per-row logs and no high-cardinality primary-key labels. Distinguish nested or overlapping timings. Do not attribute residual time by subtraction. Count actual backend lookups, decoded rows/bytes, index entries visited, operations/WAL bytes, and target-resolution misses. A cache bookkeeping counter is not backend instrumentation. Validate aggregate values and final table contents outside timing with deterministic ordered checksums or canonical multiset comparison, preserving NULL, types, and duplicate multiplicity, with appropriate float tolerance. Acceptance: a reproducible current baseline and a short table of which stages dominate #8, #11, #12, #13, and #15. Distinguish instrumented profiling runs from timing runs when overhead is material. Record the ownership/call-path map for the paths this baseline will change.

- [X] T070 [P] A2. Avoid inactive coalescing allocations, exactly handoff §A2. Files: `backend/crates/kalamdb-store/src/write_coalesce.rs` and focused store tests. Check for the active coalescing context before cloning a partition, key, or value. Allocate owned operations only when buffering is actually possible. Preserve direct writes, active buffering, cancellation cleanup, and deferred-hook semantics. Preserve or explicitly reject nested coalescing according to the existing contract. Acceptance: the inactive helper path performs no key/value-buffer allocation; existing behavior tests pass; report allocation counts and runtime for direct and coalesced paths. Do not redesign the whole coalescer. Append the evidence row to `baselines.md` (depends on T069).

**Checkpoint**: Current `dev` baseline exists. A2 is measured. Do not treat October 27 release seconds as the before column.

---

## Phase 12: Performance handoff B — transaction ownership and bulk mutation preparation

**Source**: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) Phase B.

**Goal**: Commit consumes the write set, each distinct target is decoded once per authoritative preparation, post-commit work is batched by table and scope, and claims scale with the transaction's own targets when measurement says they must.

**Rules**: One canonical mutation representation and one shared USER/SHARED lifecycle. Table differences stay in typed adapters. `ScopedTargetKey` and `TargetResolutionCache` stay; do not add a string-keyed cache. Single-row and batch operations share policy. Remove replaced preparation paths in the same slice. Correctness gates 1–8 apply, including no partial visibility, no DELETE-as-TRUNCATE, and no early acknowledgment of coalesced commits.

- [X] T071 [P] B1. Consume the write set when committing, exactly handoff §B1. Files: `backend/crates/kalamdb-transactions/src/write_set.rs`, `backend/crates/kalamdb-transactions/src/staged_mutation.rs`, the coordinator `commit` path, and focused transaction tests. This extends the fold from T035. After the transaction state becomes non-writable, fold/seal by moving payloads rather than cloning them. Release overlay maps, latest-key maps, and obsolete history before awaiting Raft/backend commit. Preserve everything needed to validate even an empty or net-no-op folded commit. Do not bypass group admission because folding produced zero writes. Keep ordered history necessary for active savepoints. Folding must not change statement counts, required events, constraints, or file cleanup. Do not erase history solely to meet a memory target. Share immutable payloads or batch slices where justified. Do not put `Arc` around every scalar. Bound retained parent batches so one small slice does not pin a huge allocation. Acceptance: tests cover insert-update-delete, delete-reinsert, repeated update, primary-key move, duplicate inserts, savepoint rollback/release, and multi-table same-group commits. Show lower peak retained bytes for 25,000 distinct keys and 25,000 mutations of one key. At the commit await, the original write-set/overlay payload must not remain accidentally owned. Pass the review gate and append the evidence row (depends on T069).

- [X] T072 B2. Resolve each distinct mutation target once per authoritative preparation, exactly handoff §B2. Files: `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, `backend/crates/kalamdb-tables/src/versioned/target.rs`, provider operations, and user/shared mutation storage paths. Reuse `ScopedTargetKey` and the existing target cache from T036. Use a distinct type for authoritative baseline versus transaction-local final state if needed. Resolve targets in bounded batches grouped by table and scope. Retain row, version, and index-relevant fields only as long as needed. Handle missing rows and tombstones explicitly. Preserve cold-row fallback and schema-version decoding. Separate safety revalidation from full payload loading. A second full decode is not required just to check a version. Prove where claims and commit ordering make reuse safe, including concurrent writers. Determine where snapshot validation actually occurs; `check_expected_max_version(version, None)` does not prove it. Prepare row and secondary-index operations together and reuse the same prior row for index extraction, files, and old-value notifications. Do not retain both complete before/after rows when the enabled effects do not need both. Compare isolated range DELETE with the current path before extending the same mechanism to UPDATE. Implement the shared policy once and keep table-specific storage adapters. Finish through the common USER/SHARED implementation and delete replaced preparation paths. Do not leave the first optimized table type on a separate permanent executor. Acceptance: counters show at most one full authoritative baseline decode per distinct target during preparation, excluding explicitly documented conflict retries. Scan and projection reads are a separate count. Selective and broad deletes preserve exact contents, snapshots, counts, and events. Batch failure yields no partial visibility. Pass the review gate and append the evidence row (depends on T071).

- [X] T073 B3. Batch post-commit work by table and scope, exactly handoff §B3. Files: `prepared_commit.rs` under the applier/commit path, provider finish methods, and the side-effect plan. Collapse repeated manifest and pending-write marks. Build vector work only for relevant indexes and notification payloads only for actual consumers or required durable delivery. Keep per-row semantic events when required. Release preparation buffers before effect dispatch when they are no longer needed. Acceptance: unwatched, non-vector workloads do O(touched table scopes) bookkeeping rather than repeated identical per-row bookkeeping. Watched, vector, and file cases preserve all required work. Failure injection proves no effects escape an unpersisted coalesced commit and no retry duplicates committed mutations. Pass the review gate and append the evidence row (depends on T072).

- [X] T074 [P] B4. ~~Make claims scale with the transaction's own targets.~~ Not required for this slice: phase H runs this only when concurrent-load evidence shows claim contention. That evidence is not in this change, so the claim map stays as it is. Original task: exactly handoff §B4. Files: `backend/crates/kalamdb-transactions/src/pk_claims.rs` and its callers. The reviewed claim map uses a single mutex and `release_owner` traverses all claims. Measure before changing synchronization. If contention or release time is material, use scoped typed keys and an owner-to-owned-keys index, with consistent acquisition and release ordering. Equal primary keys in different user scopes must not serialize unnecessarily. Handle overlapping multi-key acquisition without indefinite deadlock. Preserve the current conflict and isolation contract. Acceptance: 1/8/32-client disjoint and overlapping-key tests, cancellation/abort cleanup, no missed wakeups, and no retained owner entries after transactions finish. Releasing one owner scales with that owner's claims, not every active claim. Do not expect this change alone to improve the single-connection suite. If measurement shows the cost is not material, leave the synchronization unchanged and record that in the evidence row. Pass the review gate (depends on T069; prioritized by concurrent-load evidence, not by the single-connection suite).

**Checkpoint**: B1 is measured before B2. B2 and B3 land on range DELETE, then UPDATE. Correctness fixes stay separate from performance attribution.

---

## Phase 13: Performance handoff C — bounded columnar query and DML execution

**Source**: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) Phase C, plus the reservation-plumbing step of §D3 that the handoff requires before streaming expansion.

**Goal**: Matching, assignment, overlay merge, and hot scans stay on bounded Arrow batches. DataFusion remains the expression and visibility engine. Staging memory stays separate from scan workspace.

**Rules**: No second expression evaluator. No Halloween double-application. The unbounded scan guard stays a correctness guard. Introduce T075 before T076–T078.

- [X] T075 D3 admission plumbing, the blocking subset of handoff §D3. Files: mutation and write-set accounting, coordinator admission, and existing DataFusion/store memory controls. Define the ownership ledger categories: staged payload/history, overlay/map overhead and capacities, scan/decoded batches, target cache, encoded operations/native batch, side-effect payloads, and RocksDB memtables/cache. Distinguish RSS from tracked retained memory. Enforce per-transaction and process-wide admission before material expansion, not after constructing an oversized batch. Reuse existing configuration where it fits and document any new budget. Transfer and release reservations on commit, rollback, cancellation, errors, and savepoint rollback. No global budget lock or `DashMap` guard across await. This task introduces the plumbing C1–C3 must use. T081 completes the D3 resource tests and the rest of the acceptance text. Do not silently split commits (depends on T071).

- [X] T076 [P] C1. Keep Arrow batches through matching and assignment evaluation, exactly handoff §C1. Progress: scan and INSERT SELECT input are visited one batch at a time, assignments evaluate on that batch, and direct writes run after the scan returns. Deferred evidence: scripts #8, #11, and #12 are not re-timed on a clean tree (dirty-tree `KALAM_BASELINE=1` forbidden); the statement write set is still retained. Evidence row is in `baselines.md`. Files: `backend/crates/kalamdb-datafusion-sources/src/dml.rs`, `backend/crates/kalamdb-tables/src/utils/datafusion_dml.rs`, and provider SQL methods. Replace eager `collect -> Vec<Row> -> Arrow -> Vec<Row>` transitions with bounded `RecordBatch` streams for predicate evaluation, assignment evaluation, and INSERT SELECT input. Reuse `PreparedUpdateAssignments` and existing serialization batch APIs. Project only needed columns plus primary-key and MVCC metadata. Decode payload columns late. Convert only final mutations to the storage representation. Budget staging growth separately from scan workspace. Do not stage mutations while a scan can observe those same new mutations and update a row twice. Retain a statement-consistent input view or a budgeted/spillable target set. Acceptance: #8, #11, and #12 preserve semantics. Wide unused text and binary columns are not decoded. Input and assignment scratch allocation is bounded by configured batch-byte limits, not total matched rows. A large single row cannot bypass a row-count-only bound. Pass the review gate and append the evidence row (depends on T073, T075).

- [X] T077 [P] C2. Stream transaction overlays, exactly handoff §C2. Progress: the overlay emits one committed batch at a time, LIMIT runs after the residual filter, and the only keys kept across batches are the transaction's insert keys. The plan is split under `backend/crates/kalamdb-transactions/src/overlay_exec/`. Progress: an overlay scan reads one extra committed row per staged delete, then the overlay fetch still stops at the SQL limit. A filtered overlay scan leaves the SQL limit on the overlay, so the residual predicate runs before the fetch. An unfiltered overlay scan reads one extra committed row per staged delete. Row-map, index, and cold results leave `scan_rows_output` as session batch-size batches. Columnar hot scans now emit session batch-size chunks after the winner pass (T078, still open). Files: `backend/crates/kalamdb-transactions/src/overlay_exec/`, `backend/crates/kalamdb-transactions/src/overlay.rs`, and the table base provider. This extends T037. Preserve the Arc-backed overlay snapshot and the empty-overlay bypass. Do not clone the full map per row or hold a growing map snapshot throughout staging by accident. Merge committed batches with the immutable statement overlay by scoped primary key. Skip tombstones, patch matching base rows, append transaction-local inserts exactly once, and re-evaluate predicates whose truth may change. Handle updates that move a row into or out of a filter, including secondary-index predicates. A partial patch is not a complete row; fetch missing required base columns or use a safe fallback. Apply SQL LIMIT after visibility, overlay, and filter semantics. Keep ORDER BY semantics in DataFusion. Use a budgeted seen-key structure only where required. Do not re-materialize the entire committed table merely to track overlay keys. Acceptance: committed-row working memory scales with batch size plus necessary overlay bookkeeping, not scanned-row count. Tests cover changed predicate columns, unchanged projected columns, delete+insert of the same primary key, user isolation, NULLs, LIMIT, empty input, and repeated scans during one transaction. Pass the review gate and append the evidence row (depends on T071, T075).

- [X] T078 C3. Stream hot scans and preserve global MVCC resolution, exactly handoff §C3. Progress: a columnar hot scan still resolves versions over the whole hot range, then emits owned batches of the session batch size. `DeferredBatchExec` streams those batches so the overlay can drop each chunk. The 100,000-row guard stays. After resolved rows exist, the row-map path, including index and cold results, emits session batch-size batches. Scan-batch bytes are reserved at the shared executor collect boundary (`QueryBatchStream::collect_accounted`, T081/T087). Deferred: winner map and scalar columns still span the scan; index/cold resolution still collect before split; scripts #4 and #5 not re-measured on a clean tree. Files: base hot/deferred scan code, versioned scan adapters, the serialization batch decoder, and store iterator APIs. Decode projected hot records directly into Arrow using existing batch codecs where valid. Push filters only across operators where doing so cannot expose an older version. Do not resolve versions independently per arbitrary storage chunk. Keep global correctness through the existing DataFusion resolution or a proven ordered, budgeted plan. Use DataFusion memory reservations and spill facilities where applicable. Replace hidden unbounded-materialization assumptions with iterator continuation or explicit resource-limit errors. Do not remove or simply raise the current unbounded scan guard. Exercise data beyond its bound and long version chains. Acceptance: #4 and #5 correctness and speed are measured. Scans beyond 100,000 raw rows are complete or fail explicitly, never truncate silently. Peak working memory is bounded or accounted/spilled. Old snapshots remain correct across chunk boundaries and hot/cold transitions. Pass the review gate and append the evidence row (depends on T076, T077).

**Checkpoint**: C1 and C2 are separate reviewable slices. C3 follows them. Reservation plumbing from T075 is in place before those slices expand staging or scan ownership.

---

## Phase 14: Performance handoff D — index selection and storage memory

**Source**: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) Phase D.

**Goal**: Access choice can pick a sequential projected scan when it is cheaper, storage submission allocates less without changing atomicity, and transaction, query, and commit memory are admitted separately.

**Rules**: No second SQL optimizer. No per-chunk commit. A new atomic batch builder exists only if measurements justify it, and then it replaces the existing batch path behind one store interface. Default D2 scope is allocation reduction.

- [X] T079 [P] D1. Compare index access with sequential access, exactly handoff §D1. Equality and an equality prefix stay on the index. A leading range with no equality prefix uses the sequential scan when no row estimate exists. A selectivity estimate at or above 0.5 uses the sequential scan. Diagnostic plans record [`ScanAccess`] on `access_path`. Files: store index seek/planning, provider scan planning, and the existing statistics owner. Add inexpensive estimates of live row count, selectivity, and fetched bytes. Compare a sequential projected scan with index walk plus row fetch plus visibility resolution. Include a conservative path for missing or stale statistics. Do not scan the table to choose a plan on every request. Keep statistics scoped and bounded. Use the existing DataFusion statistics surface when practical. Expose chosen access and estimated/actual rows in diagnostic plans. Start with scalar equality and range predicates, including composite prefixes already supported. Benchmark selectivity near 0.01%, 1%, 10%, 50%, and 90%, plus narrow and wide rows. Tune from evidence. Do not branch on benchmark ids. Acceptance: selective queries keep fast indexed access. Broad range cases can select sequential access when measured cheaper. Both paths return identical data under snapshots and overlays. Measure #7, #9, #10, and #13. No unbounded statistics cache. Pass the review gate and append the evidence row (depends on T069, T073).

- [X] T080 [P] D2. Reduce storage submission allocations without changing atomicity, exactly handoff §D2. A batch reuses the column-family handle and key prefix for a partition and allocates the partition name only on the first operation for that partition. A separate atomic batch builder is not added: the handoff adds one only when the generic operation vector is a measured peak. Files: `backend/crates/kalamdb-store/src/indexed_store.rs`, `backend/crates/kalamdb-store/src/storage_trait.rs`, the RocksDB `backend.rs`, and coalescing integration. Measure allocations around `Vec<Operation>`, prefixed physical keys, per-operation partition-name strings, native `WriteBatch` construction, and prepared effect payloads. Reuse resolved partition handles within a batch without allocating a partition-name `String` on every lookup. Reserve capacities from accounted estimates and reuse scratch buffers where lifetimes permit. If the generic operation vector plus native batch is a measured peak-memory bottleneck, design an owned, backend-neutral atomic batch builder in `kalamdb-store`. Move encoded buffers into it and transfer memory reservations across ownership boundaries. Do not leak RocksDB objects into core or tables. Integrate with outer Raft coalescing so data, indexes, and required metadata remain one atomic submission. No per-chunk commit workaround. Acceptance: output operations and failure/replay behavior match the current batch path. Measure allocations, peak bytes, WAL bytes, and seconds. Implement a new builder only if measurements justify the additional interface. If introduced, it integrates with or replaces the existing batch mechanism behind one store interface. Do not retain competing commit APIs for scalar, bulk, user, shared, and coalesced writes. Pass the review gate and append the evidence row (depends on T069, T073).

- [X] T081 D3. Finish transaction, query, and commit memory accounting, exactly the remainder of handoff §D3. Progress: `Reservation::adopt`, `QueryBatchStream::collect_accounted`, `ExecutionResult::into_materialized_rows_accounted`, nested Arrow scalar sizing via `get_array_memory_size`, concurrent staged admission and wide-row bound tests in `memory_ledger`. Encoded commit batches already reserve under `EncodedBatch`. Deferred: process-level RSS vs tracked deltas and large-DML peak memory need a clean-tree T089 run. Files: mutation/write-set accounting, coordinator admission, prepared batch accounting, and existing DataFusion/store memory controls. T075 introduced admission plumbing. This task accounts strings, nested Arrow arrays, hash-map capacity, concurrent transactions, and temporary transition peaks. A shared `Arc` buffer has one owner charge or a documented conservative charge. Oversized atomic transactions fail with the existing compatible resource error, or use an explicitly designed staging/spill strategy. Do not silently split commits or claim constant memory while retaining all final operations. Acceptance: adversarial wide rows, nested values, many small rows, and concurrent transactions trigger predictable bounds. Logical live bytes return to baseline after cleanup, allowing documented allocator and RocksDB retention. The sum of admitted work stays bounded at process level. Pass the review gate and append the evidence row (depends on T075, T076, T077, T078).

**Checkpoint**: D1 and D2 follow profiles from T069. D3 resource tests are complete. B4 stays off the critical path unless concurrent evidence justifies it.

---

## Phase 15: Performance handoff E — evidence-gated investigations

**Source**: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) Phase E.

**Goal**: Decide, from measurement, whether stable secondary-index references, compact tombstones, or column-family grouping are worth a format or layout change.

**Rules**: These tasks are not prerequisites for shipping A–D. Do not implement a storage-format migration without a measured bottleneck, an ADR, and correctness tests. Leave unsupported hypotheses explicit. T082 is the investigation T056 already requires to stay out of the Row V2 flag.

- [X] T082 E1. ~~Stable secondary-index references.~~ Not required for this slice: phase H does not migrate the index format without a measured bottleneck and an ADR. T056 already records that gate. No format change here. Original task: exactly handoff §E1. Measure per-update secondary-index deletes and puts when indexed values do and do not change. If unnecessary index churn remains dominant, write an ADR comparing current version-addressed entries with stable logical-key entries and any current-version indirection. The design covers old snapshots, indexed-value changes, historical membership, uniqueness, covering queries, stale entries, cold segments, flush/compaction, migration, and mixed-version recovery. Do not skip maintenance while index entries still point at obsolete physical row versions. Require correctness tests and a measured read/write/memory tradeoff before any format migration. Record the ADR under `docs/architecture/decisions/` and cross-link T056. If measurement does not justify the change, say so and do not migrate (depends on T069, T040 when that index-role work is present; measurement can start from the current index path).

- [X] T083 E2. ~~Compact tombstones.~~ Not required for this slice: phase H stops this without a measured retention bottleneck. Tombstone payloads stay as they are. Original task: exactly handoff §E2. Measure encoded bytes and retention duration of delete tombstones that retain row fields. Explore storing only primary key, version, and deletion metadata when old values live elsewhere and notifications, recovery, indexes, files, and historical queries do not require the deleted payload there. Preserve schema-version compatibility and masking of cold rows. This needs a format and retention design before implementation. It is not clearing `fields`. If measurement does not justify the change, record that and stop (depends on T069).

- [X] T084 E3. ~~Column-family growth, compaction, and snapshot-safe reclamation.~~ Not required for this slice: phase H does not retune compaction or merge column families without a measured bottleneck. Layout stays as it is. Original task: exactly handoff §E3. Measure 10, 100, and 1,000 table/index configurations: column-family count, memtable floor, cache use, SST count and size, pending compaction bytes, stalls, and file descriptors. Do not add a shared cache. Do not switch partition layouts or merge column families without a compatibility and migration design. Locate current reclamation ownership. Test bounded maintenance with a pinned reader, then release the reader and verify reclamation catches up. Preserve tombstones needed by cold segments and unknown manifest state. A fixed timer is not a snapshot proof. Do not tune compaction or cache sizes to hide growing logical history (depends on T069, T027).

**Checkpoint**: E tasks stay open investigations until their measurements and ADRs exist. Shipping A–D does not wait on them.

---

## Phase 16: Performance handoff F — planning, cold reads, and result delivery

**Source**: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) "Additional improvements".

**Goal**: One plan build per cache identity under concurrency, no repeated prefix copies on cold Parquet scans, and SELECT results that stream to clients with backpressure.

**Rules**: These extend the shared pipeline. They do not add a specialized executor. Select them by workload evidence. F1 does not replace T050's `postgres_star` key. F1 is the implementation of T051. F2 is not a second cold-query engine. F3 starts with read-only SELECT.

- [X] T085 [P] F1. Coalesce concurrent builds of the same logical plan, exactly handoff §F1. This completes T051. Progress: one in-flight build per cache key wakes followers when the leader finishes, cancels, or the epoch changes. A rejected plan is shared with current waiters and is not retained. SELECT and DML misses join that build, then bind parameters in the caller session. Catalog-function plans cancel the lead instead of publishing it. Deferred evidence: peak planning bytes, planning CPU, and p95 latency are not measured on a clean tree. Files: `backend/crates/kalamdb-plan-cache/src/lib.rs` and the SELECT/DML executor paths in `backend/crates/kalamdb-core/src/sql/executor/sql_executor/planning.rs`. `try_lead`, `finish_lead`, and `cancel_lead` already exist. Awaiters reuse the completed immutable template, then bind their own parameters and execution context. Keep planning outside locks. Use asynchronous completion and wakeup. Do not poll `try_lead` in a loop. Leader cancellation, build errors, non-cacheable plans, admission rejection, and epoch changes release waiters and in-flight entries. Share a transient successful result with current waiters even when retention admission rejects it, subject to valid identity, epochs, and memory accounting. Never share session-dependent catalog-function results or captured transaction or user state across requests. Acceptance: 32 synchronized requests for one uncached eligible query cause one planning attempt, and all receive correct independently bound results. Different keys still plan concurrently. Tests cover cancellation, DDL/permission invalidation, and oversized or non-cacheable plans without leaks or hangs. Report peak planning bytes, planning CPU, and p95 latency against the independent-build baseline. Pass the review gate and append the evidence row (depends on T050, T069).

- [X] T086 F2. Remove repeated concatenation from cold Parquet scans, exactly handoff §F2. Progress: `ColdBatchRead` holds one unread Parquet batch and blocks until MVCC merges and drops it. Count scans fold metadata one batch at a time and keep tombstones until the end. Primary-key cold lookup keeps only the newest matching live row. Up to 8 file opens stay in flight. Deferred evidence: copied bytes, peak memory, and seconds are not measured on a clean tree. Files: `backend/crates/kalamdb-tables/src/manifest/planner.rs` (`append_record_batch`, `scan_parquet_files_async`). First eliminate repeated whole-prefix copying. If the current return contract requires one batch, use budgeted accumulation plus a single final concatenation as an intermediate change, and report that this still retains the entire result. Prefer extending the existing cold scan interface to pass bounded batches into the common DataFusion/MVCC pipeline, coordinated with T078. Do not create another cold-query engine. Preserve projection, schema evolution, file pruning, error behavior, global version resolution, and snapshot lifetimes. Keep file-open and prefetch concurrency bounded by bytes as well as counts. Acceptance: tests at 1, 10, 100, and 1,000 input batches, both at fixed total rows and at fixed rows per batch. Output matches. Measure copied/allocated bytes, peak memory, and seconds. Previously accumulated data is not copied again on every batch. Do not accept a faster run caused by reading fewer files or skipping rows. Pass the review gate and append the evidence row (depends on T078).

- [X] T087 F3. Stream SELECT results to clients with backpressure, exactly handoff §F3. Progress: `QueryBatchStream` (mpsc buffer=1) + `ExecutionResult::StreamingRows`; wire async encoder pulls the stream; HTTP/cluster/functions/live materialize via `into_materialized_rows`; `collect` / `collect_accounted` adapters for complete-result callers. Deferred evidence: large SELECT first-row/total/peak-bytes timings and throttled-client runs await a clean-tree measurement. Files: `backend/crates/kalamdb-core/src/sql/context/query_stream.rs`, `execution_result.rs`, `planning.rs`, `backend/crates/kalamdb-postgres-wire/src/row_encoder.rs`.

**Checkpoint**: F1 can land after T069 without waiting for columnar scans. F2 follows C3. F3 follows the shared stream and budget contracts.

---

## Phase 17: Performance handoff verification

**Source**: [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) "Verification and quantitative acceptance" and "Execution order and reporting".

**Goal**: Prove the shipped slices on USER and SHARED tables, and report actual seconds and memory. Do not call the program complete after only small allocation fixes.

- [ ] T088 Correctness matrix, exactly the handoff "Correctness matrix". Progress: unit suites for shipped A–F slices pass locally (`memory_ledger`, `accounted_collect`, prior overlay/plan-cache/dml gates). Full matrix + CLI smoke still require a running server and a clean checkout; not closed here. Exercise USER and SHARED tables, with and without scalar indexes, hot-only and mixed hot/cold, watcher off and on, distinct and repeated keys, primary-key changes, indexed and non-indexed updates, narrow, wide, and nested rows, commit, rollback, and savepoints, conflict waits, and restart/replay. Cover file and vector paths when their finish behavior changes. Test errors before submission, backend write failure, cancellation, and restart after persistence but before acknowledgment or effects. Use targeted `cargo nextest run` suites for each shipped slice. Add `#[ntest::timeout(...)]` from observed healthy runtime × 1.5. Batch compile feedback and fix from one captured output. Run required backend and CLI smoke tests before any separately authorized commit. Architecture changes update `docs/architecture/`. New user-facing settings or behavior update canonical skills and mirrors per `AGENTS.md`. Each shipped slice from T070–T081, and any shipped F slice, has its tests here or in that slice's own task (depends on T070, T071, T072, T073, T076, T077, T078, T081).

- [ ] T089 Performance and memory acceptance, exactly the handoff "Performance and memory acceptance". Progress: blocked on a clean-tree `benchv2/pgwire-speed` five-suite run (`KALAM_BASELINE=1` forbidden on dirty tree). No invented medians. Preserve all 16 script results. Report per-script and total medians and ranges, in seconds, plus measured stage counters. No improvement claim from unit tests or a single timing sample. Goals, not permission to weaken correctness: at least 20% lower current-baseline total and 30% lower combined #8/#11/#12/#13/#15 time, and at least 30% lower peak tracked transient memory on the selected large-DML workload. Report tracked bytes and RSS. Flag regressions above max(5%, 0.005s) in any script median and repeat enough to distinguish noise. Do not average away a point-query regression. Add 1/8/32-client realistic prepared CRUD workloads with full parameters and results and matched settings on both engines. Report throughput and p50/p95/p99 latency separately. Run a minimum 300-second mixed CRUD churn series on a bounded live key set with production-like flushing, and a separate pinned-snapshot phase followed by unpin and reclamation. Record throughput, latency, RSS, tracked memory, memtables/cache, WAL/SST/disk footprint, and compaction pressure. After release, investigate sustained growth. If a goal is not met, say so and name the measured remaining work. Write the checkpoint into `baselines.md` (depends on T069, T088, and the shipped A–D slices).

**Checkpoint**: Report changed behavior, source files, actual timings, allocation and memory deltas, correctness checks, the architecture review gate, and the next bottleneck. E tasks still require their own measured bottleneck and ADR.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: Start immediately. T002 waits on T001.
- **Foundational (Phase 2)**: Starts after T002. Blocks every user story.
- **User Story 2 (Phase 3)**: Starts after Phase 2. Blocks Story 1's engine tasks (`T029` needs `T014` and `T016`). `T063`–`T066` finish inside Story 2.
- **User Story 1 (Phase 4)**: Starts after `T014` and `T027`. Folding (`T035`) waits until `T032` passes. `T067` waits on `T030`, `T013`, and `T065`.
- **User Stories 3–6**: Start after their named dependencies. Stories 4, 5, and 6 can proceed in parallel once Story 2's engine and flush job exist.
- **User Story 7**: Starts after Phase 2. It does not require Stories 3–6.
- **Polish**: Starts after the stories being shipped. `T061` waits on the P0 and P1 gates that are in scope. It does not wait on the performance handoff.
- **Performance handoff (Phases 11–17)**: Starts at T069 on the current tree. It does not wait for unfinished Story 4–7 tasks except where a task names one. Order is the handoff execution order: T069; then T070 and T071; then T072 and T073; then T075 before T076/T077 and T078; then T079/T080 and T081; T074 only when concurrent evidence justifies it; T088 and T089 after the shipped A–D slices. T085 may follow T069. T086 follows T078. T087 follows T075 and T078. T082–T084 are conditional.

### User Story Dependencies

- **User Story 2 (P0)**: After Phase 2. No dependency on other stories.
- **User Story 1 (P0)**: After `T014` from Story 2, plus its own snapshot-pin tasks. This is the spec's required order: shared processing first, then uncoalesced atomic commit.
- **User Story 3 (P1)**: After `T030`. Folding is invalid before uncoalesced atomic commit is proven.
- **User Story 4 (P1)**: After `T012`. Snapshot retention in `T043` also uses `T027`. Live query for system writes (`T068`) also waits on `T065`.
- **User Story 5 (P1)**: After `T023` and `T004`.
- **User Story 6 (P1)**: After Phase 2. Independent of the storage engine.
- **User Story 7 (P2)**: After Phase 2. Independent of Stories 3–6. T056 stays a gate. T082 is the measurement and ADR for that gate, not a format migration inside Row V2.
- **Performance handoff**: Specified by [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md). T069 blocks measurement-dependent slices. T070 and T071 can run together after T069. T072 waits on T071. T073 waits on T072. T075 waits on T071 and blocks T076, T077, and T087. T076 and T077 can run together after T073 and T075 respectively. T078 waits on T076 and T077. T079 and T080 can run together after T069 and T073. T081 waits on T075 and T076–T078. T074 waits only on T069 and on concurrent-load evidence. T085 waits on T050 and T069 and completes T051. T086 waits on T078. T088 waits on the shipped A–D slices. T089 waits on T069 and T088. T082–T084 do not block T088 for A–D.

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
- After T069, T070, T071, T074, and T085 can run together. T076 and T077 can run together after T075. T079 and T080 can run together after T073.

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

### Performance handoff

1. T069 baseline, correctness checks, and the ownership/call-path map. Revalidate folding, target cache, batching, and overlay.
2. T070 and T071. Measure before T072.
3. T072 and T073 on range DELETE, then UPDATE.
4. T075, then T076 and T077, then T078.
5. T079 and T080 from profiles. Finish T081. Start T074 only if concurrent measurements show claim cost.
6. T088 and T089. T082–T084 only with a measured bottleneck and an ADR. T085 may follow step 1. T086 follows step 4. T087 follows the stream and budget contracts from steps 4 and 5.

---

## Notes

- `[P]` tasks use different files and do not wait on an unfinished task.
- System catalog `get`/`put`/`scan_index`/`update_with_old`/`delete` stay off DataFusion. They call the shared index and schema-history code. They do not grow a second copy of it.
- `#[table]` and `build_record_batch!` stay. Provider-impl macros and hand-written duplicate schemas go.
- `pg_catalog` `compute_batch` bodies, `SharedTableAuthorization`, and the hot-scan, covering-aggregate, and equality-update bodies stay the reference. Move or call them. Do not rewrite them.
- Shared tables keep forced row-level security. A later view over a shared table uses `KalamTableProvider` and that same security.
- Commit after each original story task or a coherent group only when the user asks. Do not start Story 3 folding until Story 1's uncoalesced commit tests pass.
- Phases 11–17 implement [dml-storage-performance-handoff.md](dml-storage-performance-handoff.md) exactly. That file is the same text as `docs/plans/2026-10-30-dml-storage-performance-handoff.md`. Do not commit those slices unless the user explicitly asks. Do not reset the dirty tree. Evidence rows go in `baselines.md`. Do not open a second planning document for this program.
