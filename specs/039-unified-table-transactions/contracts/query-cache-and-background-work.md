# Contract: Query Plan Cache, AST Semantics, and Background Work Budgets

**Feature**: `039-unified-table-transactions`  
**Covers**: FR-022–FR-034, SC-006–SC-009, SC-011

---

## 1. Contextual Plan Cache & Singleflight Contract (FR-026–FR-028, SC-008)

### `PlanCacheKey` Completeness
Every `PlanCacheKey` constructed by `SqlExecutor` MUST include:
- `namespace: SchemaId`
- `role: Role`
- `sql: String` (or normalized literal-parameterized template + scalar type tags)
- `search_path: String`
- `postgres_star: bool` (`true` when `PostgresStar` session extension is present on PGWire/bridge sessions; `false` on Kalam HTTP SQL sessions where `_version`/`_deleted` are included in `*`)
- `parameter_oids: Vec<u32>`
- `session_settings: String` (canonical sorted `key=value` pairs)
- `dependency_epoch: u64` and `permission_epoch: u64`
- `principal: Option<UserId>` (populated whenever the plan binds principal-specific state)

### Singleflight Miss Coordination
- When `PlanCache::get(&key)` misses:
  1. Caller invokes `PlanCache::acquire_or_wait(&key) -> PlanSlot`.
  2. `PlanSlot::Leader(LeaderGuard)`: caller builds the `LogicalPlan` outside the cache mutex, then calls `guard.finish(plan)` which admits the plan (if epochs are still current and byte budgets permit) and wakes all waiting followers. If `LeaderGuard` is dropped without `finish` (error, cancellation, panic), it removes the inflight entry and wakes followers so they can retry or plan independently.
  3. `PlanSlot::Follower(Notified)`: follower awaits completion (bounded by timeout), then re-reads `PlanCache::get(&key)`.

### Scoped Invalidation
- Storage flush and compaction (`kalamdb-flush`) MUST NOT clear `PlanCache` or `InsertMetadataCache`.
- Table-specific DDL/policy changes invalidate entries whose `referenced_tables` contain the target `TableId` (or bump the relevant epoch), leaving unrelated cached plans intact where safe.

---

## 2. Structured AST Semantics Contract (FR-029)

1. **`RETURNING` Result Description**:
   - `kalamdb_sql::returning_describe_statement(statement: &Statement) -> Option<Statement>` constructs a `sqlparser::ast::Statement::Query` directly from the parsed `Insert` / `Update` / `Delete` AST's table `ObjectName` and `returning: Vec<SelectItem>` with `WHERE FALSE`.
   - No `format!("SELECT {projection} FROM {from} WHERE FALSE")` string interpolation or re-parsing is permitted on the wire describe path.
2. **Target Table Extraction**:
   - Target `TableId` extraction for stream autocommit detection and transaction routing uses the parsed `Statement` AST (`extract_dml_table_id_from_statement` / `extract_query_table_id_from_statement`), never `sql.to_ascii_lowercase().find(" from ")` or `.split('.')`.

---

## 3. Background Flush, Compaction, and Transfer Budget Contract (FR-022–FR-025, FR-033, FR-034)

1. **Flush Lock Discipline (`ManifestService` & `FlushScopeWriter`)**:
   - `ManifestService::mark_pending_write` MUST NOT block on bulk Parquet write/upload/hook locks.
   - `FlushScopeWriter::write_scope` holds the scope publication lock only during:
     - Phase A: batch filename reservation and manifest snapshot read.
     - Phase C: final manifest segment addition + `manifest.json` persistence after the `.parquet` file is already written and renamed.
   - Phase B (Arrow batch encoding, Parquet compression/upload, temp-to-final rename) and post-publication hooks run outside the scope lock.
2. **Scope Lock Reclamation**:
   - Inactive `flush_scope_locks` entries with zero active holders/waiters are reclaimed automatically so high-cardinality user-scope churn cannot grow memory unboundedly.
3. **Streaming Transfer Archives (`kalamdb-transfer` & `kalamdb-jobs`)**:
   - `kalamdb-transfer` MUST NOT depend on `kalamdb-core` (`Cargo.toml` dependency removed; uses `TransferError`).
   - Table export, table import, and user data export stream segment files through bounded buffers inside `spawn_blocking` rather than accumulating all segment `Vec<u8>` payloads in memory simultaneously.
4. **Typed Job Orchestration (`AppContext`)**:
   - `AppContext` exposes a typed job manager/dispatcher trait rather than requiring callers to downcast `Arc<dyn Any>` for standard job operations.
