# Shared-table RLS Implementation Plan

**Date:** 2026-08-20  
**Status:** ACCESS_LEVEL removed; FORCE RLS default-deny is in place. Security matrix, live grant/revoke fail-closed, ON CONFLICT reject, and CLI e2e coverage are in. Remaining work is strategy/EXPLAIN metrics, covering indexes, and performance benches — not ACCESS_LEVEL.  
**Cursor plan:** `.cursor/plans/shared_table_rls_6ffd6f10.plan.md`

**Goal:** PostgreSQL-shaped shared-table RLS compiled to an authorization IR, then executed with the cheapest safe strategy — PointGuard/MultiGuard, DataFusion 55 LeftSemi/HashJoin, or a generation-aware authorization cache. RLS never runs policy SQL per row or per live event. Identity is bound after the user-independent plan cache. Mutable-column RLS applies only after MVCC winner selection.

**Architecture:** Developers write `CREATE POLICY` / `USING` / `WITH CHECK`. KalamDB compiles that to `RowLocal` or `AuthorizationRelation` semantics, binds `CURRENT_USER` at execution/subscription time, and selects PointGuard, MultiGuard, DataFusion semi-join, or a cached authorization set. Live fan-out does zero SQL and zero DataFusion.

**Tech stack:** Rust workspace, DataFusion 55.0.0, existing `SharedTableProvider` / `kalamdb-datafusion-sources` / `IndexedEntityStore` / live `RowFilter` fan-out.

---

## Implementation todos

1. Persist `TablePolicy` definitions (not execution plans); `system.table_policies`; policy/schema generations; reverse dependency registry; DROP TABLE cleanup
2. Parse CREATE/ALTER/DROP POLICY; PostgreSQL-shaped permissive syntax; reject RESTRICTIVE and ACCESS_LEVEL
3. Compile USING/WITH CHECK to `PolicyProgram` IR (`RowLocal`, `AuthorizationRelation`); no `snapshot_sql`; dependency DAG and invalidation metadata
4. Select PointGuard, MultiGuard, DataFusionSemiJoin, or CachedAuthorizationSet from bound policy plus safe client predicates; no hardcoded IN vs HashSet thresholds
5. Bind principal after plan-cache lookup; post-MVCC RLS barrier; late-materialize RLS keys; reuse DF 55 LeftSemi/HashJoin dynamic filters for broad membership
6. Reuse `IndexedEntityStore` for PointGuard/prefix membership lookups; warn or require covering `(principal, relation_key)` indexes; do not invent a parallel RLS index system
7. One evaluator for INSERT/UPDATE/DELETE/ON CONFLICT/RETURNING/batch/transaction paths; reject unsupported write paths rather than bypass
8. Cache `Arc<AuthorizationSet>` by policy+principal+dependency generation; commit-ordered invalidation; fail closed on stale positive auth
9. Bind evaluator at subscribe; 0 SQL/0 DF in fan-out; PointGuard for equality subscriptions; inverted auth index optional; old/new visibility transitions
10. ACCESS_LEVEL is not SQL. CREATE/ALTER reject it. FORCE RLS default-deny; grants via CREATE POLICY only
11. Deny User/Service raw shared file download; audit ON CONFLICT, exports, ingestion, maintenance, backup
12. EXPLAIN strategy (PointGuard vs DF semi-join); DF/Kalam metrics; missing-index warnings; never print another principal's keys
13. Security matrix, MVCC winner vs RLS, PointGuard, plan-cache isolation, live grant/revoke e2e, ON CONFLICT; DF EXPLAIN fixtures; performance benchmarks
14. SQL reference, RLS vs WHERE, index guidance, migration, live semantics, kalamdb-skills mirrors

---

## Goal

Developers write PostgreSQL `CREATE POLICY` / `USING` / `WITH CHECK`. KalamDB compiles that to **authorization semantics**, then picks the cheapest **execution strategy**. RLS should usually **eliminate work**, not add a per-row policy interpreter.

Checkout fact: workspace pins **Apache DataFusion 55.0.0** (`Cargo.toml` `datafusion = { version = "55.0.0", ... }`, `Cargo.lock` checksum `96f76f0167ed0842b29a3d1e41be3c034c0a46409a3c703cc4cc84ee8c24abf4`). APIs below were verified against `datafusion-*-55.0.0` and `datafusion-physical-plan-55.0.0`, not from memory.

Invariant:

```text
user-visible rows = RLS-authorized MVCC winners AND client query/filter
```

A malicious `OR 1=1` must never change the first term.

---

## Inspected baseline (do not reimplement)

### DataFusion 55.0.0 (reuse)

- `EXISTS` / `IN` subquery → `LeftSemi` / `LeftAnti`: `datafusion-optimizer-55.0.0` `DecorrelatePredicateSubquery`. Broad membership SQL should be a semi-join, not a custom nested loop.
- `HashJoinExec` + `DynamicFilterPhysicalExpr`: `datafusion-physical-plan-55.0.0` `joins/hash_join/exec.rs`. Build side becomes a dynamic probe filter.
- Adaptive IN vs hash membership: `PushdownStrategy::{InList, Map}` with `hash_join_inlist_pushdown_max_size=128KiB` and `hash_join_inlist_pushdown_max_distinct_values=150`. Do **not** write Kalam `if n < N { IN } else { HashSet }` on the SQL join path.
- Join/TopK/Agg dynamic filter pushdown: `enable_dynamic_filter_pushdown`, `enable_join_dynamic_filter_pushdown` default **true**. Keep enabled.
- Parquet bloom + page-index pruning: Kalam already sets these in `backend/crates/kalamdb-core/src/sql/datafusion_session.rs`.

Kalam `SessionConfig` does **not** override the hash-join IN thresholds; DF 55 defaults apply. First EXPLAIN/benchmark task: confirm a `LeftSemi` on `group_members` actually attaches `PushdownStrategy` to the `group_messages` scan.

### KalamDB (reuse / extend)

- [`SharedTableProvider`](backend/crates/kalamdb-tables/src/shared_tables/shared_table_provider.rs): FORCE RLS after MVCC winners; System/DBA bypass; User/Service default-deny without `CREATE POLICY`.
- `backend/crates/kalamdb-datafusion-sources/src/lib.rs`: shared `ExecutionPlan` scaffolding, pruning descriptors, deferred MVCC exec.
- `backend/crates/kalamdb-datafusion-sources/src/pruning.rs` `mvcc_filter_evaluation`: **exact** filters run on **resolved winners**; **inexact** pre-winner pruning is only PK equality, `_seq` bounds, `_deleted`. Mutable `group_id IN (...)` is **not** in that inexact set today — keep it that way.
- `backend/crates/kalamdb-tables/src/utils/base.rs` `compute_metadata_only_cold_columns` / `compute_cold_columns`: already metadata-first / projection-aware cold reads.
- `backend/crates/kalamdb-store/src/indexed_store.rs`: prefix `scan_by_index`, `find_best_index_for_filters`. Compound example: `backend/crates/kalamdb-system/src/providers/jobs/jobs_indexes.rs` `(status, created_at, job_id)`.
- Shared tables today: `backend/crates/kalamdb-tables/src/shared_tables/pk_index.rs` only `(pk, seq)`. There is **no** general `CREATE INDEX` for `(user_id, group_id)` on shared tables (dialect `CREATE INDEX` is vector cosine). PointGuard is **not** O(1) unless we cover the membership lookup with PK/index/`AuthorizationSet`.
- Plan cache: `backend/crates/kalamdb-core/src/sql/executor/sql_executor.rs` `PlanCacheKey` = namespace + role + SQL. User-agnostic on purpose.
- Live: `backend/crates/kalamdb-live/src/manager/queries_manager.rs` `compile_subscription_filter` → `kalamdb-row-filter`; fan-out `backend/crates/kalamdb-live/src/notification.rs` `dispatch_chunk`. Subqueries rejected. Catch-up uses SQL/`scan`.
- DML: `backend/crates/kalamdb-core/src/sql/executor/transaction_batch_insert.rs` ON CONFLICT paths.

---

## Product model

Always-on FORCE RLS for shared tables. No ACCESS_LEVEL. System/DBA bypass. User and Service are subject to RLS. Default deny. `PUBLIC` = every role subject to the table (use `TO user, service` for authenticated-only). Permissive OR. `AS RESTRICTIVE` errors. ALTER POLICY supported. Policy DDL: System/DBA/Service.

Command semantics (SELECT USING, INSERT WITH CHECK, UPDATE old USING + new CHECK, DELETE USING) stay PostgreSQL-shaped. One semantic evaluator for SQL, pgwire, HTTP, transactions, catch-up, live.

---

## Compiler: syntax ≠ execution

At `CREATE POLICY`, compile to IR. Persist **SQL text + IR metadata**, never DataFusion physical plans or `snapshot_sql: String`.

```rust
enum PolicyProgram {
    RowLocal { expr: BoundExprShape }, // owner_id = CURRENT_USER(), literals, AND/OR/NOT
    Membership(AuthorizationRelation),
}

struct AuthorizationRelation {
    protected_table: TableId,
    protected_keys: Vec<ColumnId>,      // group_messages.group_id
    relation_table: TableId,            // group_members
    relation_keys: Vec<ColumnId>,       // group_members.group_id
    principal_column: ColumnId,         // group_members.user_id
    principal: PrincipalExpr,           // CURRENT_USER
    static_predicates: Vec<ScalarPred>, // optional extra relation filters
    dependencies: Vec<TableId>,
    invalidation: InvalidationStrategy, // TargetedPrincipal | WholePolicy
}
```

`EXISTS (... correlated ...)` and `protected_key IN (SELECT relation_key FROM relation WHERE principal = CURRENT_USER())` **normalize to the same** `AuthorizationRelation` or fail at CREATE POLICY.

Unsupported: self-reference, cycles, extra unrelated tables, aggregates/windows, volatile UDFs, non-normalizable correlation.

The IR describes **what must be true**, not **how** to evaluate it.

---

## Strategy selection (after identity bind)

```rust
enum AuthorizationStrategy {
    PointGuard,              // protected key already a safe scalar/param
    MultiGuard,              // protected key IN (safe scalars/params)
    DataFusionSemiJoin,      // broad query: DF LeftSemi / HashJoin
    CachedAuthorizationSet,  // reuse Arc<HashSet> for live + repeated SQL
}
```

Do **not** hard-code cardinality cutoffs until benchmarks. DF already chooses InList vs hash map on the join path.

```text
cached logical plan (no user_id, no folded CURRENT_USER)
        ↓
session bind → BoundPolicy(principal)
        ↓
extract SECURITY-SAFE client constraints only
        ↓
strategy
```

Safe constraints that may participate in PointGuard/MultiGuard (leakproof, no UDF):

```text
protected_column = literal
protected_column = bound parameter
protected_column IN (literal | parameter, ...)
AND of the above
```

Everything else stays **above** the RLS barrier.

---

## Fast path 1 — PointGuard

```sql
SELECT * FROM app.group_messages WHERE group_id = $1;
-- policy: Membership on group_id via group_members
```

Physical target:

```text
authorization.exists(principal, $1)
     false → EmptyExec
     true  → ordinary optimized group_messages query (same as no RLS, plus post-MVCC barrier no-op)
```

Target cost: **one cached or indexed membership probe + normal query**. No `group_members` scan, no hash join.

Backing access (pick first that exists; do not claim O(1) otherwise):

1. `AuthorizationSet` cache hit → `HashSet::contains` (true O(1) memory).
2. Membership relation unique/PK covering `(user_id, group_id)` → `SharedTablePkIndex` / `IndexedEntityStore` point lookup, then MVCC-resolve **that** membership row.
3. Compound `IndexDefinition` `(user_id, group_id)` prefix-compatible with `scan_by_index` (same pattern as jobs status index).
4. Fallback: DataFusionSemiJoin (correct, slower).

CREATE POLICY **warns** (does not auto-create) if neither PK nor covering index exists for `(principal, relation_key)`.

**Decision record**

- Correctness: authorize one key, then run the user query unchanged.
- Security: only leakproof equality/IN; UDFs never run before the probe.
- Performance: ~1 lookup when indexed/cached.
- DataFusion reused: none on the probe; normal scan/filter/parquet for the messages query.
- KalamDB custom: constraint extractor + exists() against index/cache.
- Fallback: SemiJoin.
- Benchmark: PointGuard cache hit vs index hit vs no RLS vs SemiJoin.

---

## Fast path 2 — MultiGuard

```sql
WHERE group_id IN ($1, $2, $3)
```

Batch `requested ∩ authorized` (one index prefix / cached set / batched PK gets — **not** one SQL per key). Empty → `EmptyExec`. Narrow the protected scan to the intersection with the same leakproof whitelist.

**Decision record:** same as PointGuard; benchmark 1/5/25/150 keys.

---

## General path — DataFusion semi-join

```sql
SELECT * FROM group_messages ORDER BY created_at DESC LIMIT 100;
```

Inject a **security-barrier** authorization join, not a correlated per-row `EXISTS` interpreter.

Preferred physical shape (let DF 55 produce it):

```text
HashJoinExec LeftSemi
  probe: group_messages  (post-MVCC winners; see MVCC section)
  build: group_members WHERE user_id = <bound principal>
```

`DecorrelatePredicateSubquery` already rewrites this class of predicate. Verify with EXPLAIN that Kalam's injected plan hits `HashJoinExec` + `PushdownStrategy::InList` (≤150 distinct / 128KiB) or `Map` (hash membership). If the injected shape fails to decorrelate, fix the **injection shape** (smallest change), do not rewrite the join engine.

CURRENT_USER is bound to a **session scalar** in the **build-side** filter (`user_id = 'alice'`), never folded into the cached logical plan template.

Do **not** add `RlsMembershipFilterExec` for this SQL path unless EXPLAIN/benchmarks prove HashJoinExec cannot attach dynamic filters to Kalam MVCC scans. If needed, the smallest integration is teaching `kalamdb-datafusion-sources` to consume DF dynamic filters the way Parquet already can — not a second hash table.

**Decision record**

- Correctness: semi-join ≡ EXISTS.
- Security: build side is principal-restricted; probe rows are MVCC winners; client UDFs stay above barrier.
- Performance: DF adaptive IN/hash + existing parquet bloom/page pruning.
- DataFusion reused: decorrelate, HashJoinExec, DynamicFilterPhysicalExpr, InList/Map pushdown, join selection.
- KalamDB custom: inject bound build-side scan + barrier placement relative to MVCC.
- Fallback: CachedAuthorizationSet as build-side in-memory table if relation scan is expensive.
- Benchmark: LeftSemi vs optimized hash join vs cache hit; 1…10k auth keys; 100k/1M/10M protected rows.

---

## MVCC security (hard)

Do **not** add RLS predicates on **mutable** columns to `is_mvcc_source_pruning_filter`.

```text
PK same row: commit 10 group_id=A, commit 20 group_id=B
Alice authorized for A only
If group_id IN AliceGroups filters versions before winner selection:
  commit 20 dropped, commit 10 wrongly visible
```

Required order:

```text
physical versions
  → MVCC winner (PK, commit_seq, seq, deleted)
  → RLS (PointGuard / MultiGuard / SemiJoin / cache)
  → untrusted client predicates / UDFs
```

PK equality / `_seq` / `_deleted` may still prune **before** winners (already true). RLS keys that are the PK are identity, not mutable authorization columns.

**Late materialization** (performance, not a license to skip winners):

```text
read winner metadata (PK, commit_seq, seq, deleted)
  → winners
  → decode only RLS key columns (extend compute_cold_columns)
  → authorize
  → decode remaining projection only for authorized winners
```

Reuse existing metadata-only cold helpers; evaluate covering index values if RLS keys are already in index_value(). Do not decode full rows then RLS-filter as the only path.

**Decision record:** custom Kalam (MVCC); DF pruning applies to cold Parquet of **winners'** files after authorization keys are known. Benchmark hot-only vs hot+cold vs parquet-heavy with/without late materialization.

---

## Security barrier

Not `rls_pred AND user_pred` with unrestricted reorder.

```text
storage pruning (PK/seq/deleted only)
  → MVCC winners
  → RLS (PointGuard / SemiJoin / cache)   ← barrier
  → client WHERE / JOIN / UDF / projection
```

If DF would place a user UDF under the join, insert a thin `RlsBarrierExec` (pass-through after authorization) rather than a full custom join. Prefer zero extra nodes when the physical plan already has LeftSemi above the deferred MVCC source and below user FilterExec.

---

## Caches (semantics, not “this SQL is allowed”)

1. **Compiled policy** `(table_id, policy_generation, schema_generation)` → `Arc<CompiledTablePolicies>`.
2. **Bound policy** per execution/subscription (principal scalar). Never mutates global plan cache.
3. **AuthorizationSet** `(policy_id, policy_version, principal_id, relation_generation)` → `Arc<AuthorizationSet>`. Shared by broad SQL, PointGuard contains(), and live.

Do not cache “query Q is authorized” unless the key includes principal, policy generation, dependency generation, and the safe extracted keys.

Invalidation: reverse `relation_table → policies`. Targeted when `principal_column = CURRENT_USER` is proven. Membership INSERT/DELETE/UPDATE updates generations **before** later protected DML is dispatchable. Stale positive → **deny**. TTL is not correctness.

Plan cache stays user-independent. Policy DDL bumps generation so the next execution rebinds. `CURRENT_USER` UDF stays `Stable` for SQL but **must not** constant-fold into cached templates; bind on the build side / BoundPolicy instead.

---

## Indexes (no parallel RLS store)

Reuse `IndexedEntityStore`.

Ideal membership key: `(user_id, group_id)` supporting:

- exact `(Alice, A)` → PointGuard
- prefix `(Alice)` → fill AuthorizationSet / SemiJoin build

Options (choose after write-amp vs lookup latency benchmarks; default order):

1. Application unique/PK on `(user_id, group_id)` so existing PK index works.
2. Internal `IndexDefinition` on the membership shared store, same as jobs compound index (extra write amp on membership DML only).
3. Warn-only + SemiJoin/cache until an index exists.

Do not auto-CREATE INDEX in v1. Vector `ALTER TABLE … CREATE INDEX … USING COSINE` is unrelated.

---

## DML

Same evaluator: `visible_old_row` / `check_new_row`. Defaults (`DEFAULT CURRENT_USER()`) before WITH CHECK. Batch atomic. ON CONFLICT INSERT vs UPDATE branches both go through it (`transaction_batch_insert.rs`). Unsupported path → error, not bypass.

PointGuard applies to INSERT/UPDATE CHECK when the new row’s protected key is a bound scalar (`group_id` in the inserted tuple).

---

## Live (0 SQL in the hot path)

Subscribe: bind `BoundPolicy` + share `Arc<AuthorizationSet>` / PointGuard result on `SubscriptionHandle`.

- `WHERE group_id = $1` → PointGuard once; invalidate on membership change for that principal+key.
- Broad subscribe → `AuthorizationSet.contains` or inverted `(policy_id, group_id) → handles`.

Notify loop: **0 SQL, 0 DataFusion planning, 0 membership scans**.

`visible = rls AND client RowFilter`. UPDATE old/new: none / INSERT / UPDATE / DELETE-tombstone (no unauthorized new columns). DELETE payload: PK + system ids.

Grant: later events only (no historical backfill v1). Revoke: no subsequent pushes; fail closed.

Catch-up is the SQL/provider path (same RLS), never a weaker copy.

---

## ACCESS_LEVEL, files, catalog

`ACCESS_LEVEL` is not a table option. CREATE/ALTER TABLE reject it. Shared tables are
FORCE RLS default-deny until `CREATE POLICY`. Catalog JSON may still deserialize a
legacy `access_level` field and ignore it.

Raw shared Parquet: System/DBA only. User/Service denied even with SELECT policies.

`pg_catalog` may list a table while RLS returns 0 rows.

---

## Custom vs DataFusion (explicit)

**Necessary KalamDB code**

- Policy DDL + IR compiler + generations
- PointGuard/MultiGuard extractor (DF does not know “this equality is the RLS key”)
- Principal bind after plan cache
- Post-MVCC barrier placement / late RLS-key decode
- AuthorizationSet cache + commit-ordered invalidation
- Live evaluator + visibility transitions
- Optional inverted live index
- Optional compound `IndexDefinition` on membership tables

**Do not build**

- Per-row EXISTS interpreter
- SQL-path `if n < N { IN } else { HashSet }`
- Custom hash-join / decorrelation
- `snapshot_sql` as the execution model
- `RlsMembershipFilterExec` unless DF dynamic filters cannot attach to MVCC scans (prove with EXPLAIN first)
- A second index stack unrelated to `IndexedEntityStore`

---

## Performance targets

- Row-local `owner_id = CURRENT_USER()`: ~ vectorized filter after bind (no UDF per row).
- Membership + `WHERE group_id = $1`: **one** cache/index exists() + normal query.
- Broad small set: DF InList dynamic filter (default ≤150 distinct).
- Broad large set: DF hash-map membership, not a giant logical `IN`.
- Live event: O(1) contains/routing.

---

## EXPLAIN

```text
RlsGuard  strategy=PointGuard  policy=member_can_read  auth_cache=hit
RlsAuthorizationJoin  strategy=DataFusionHashSemiJoin  build_rows=12  dynamic_filter=true
RlsAuthorization  strategy=MultiGuard  requested_keys=5  authorized_keys=3
```

Never print another principal’s keys.

Metrics: compiled/auth-set hit/miss, rebuild_ms, invalidations, live checks, index vs cache probe, rows materialized vs scanned.

---

## Tests and e2e

- Default deny for User/Service; System/DBA bypass
- Query bypass: `OR 1=1`, omitted WHERE, nested queries, prepared params
- **MVCC:** update `group_id` A→B; Alice in A only must **not** see the old version; e2e SQL + live tombstone
- **PointGuard:** EXPLAIN shows PointGuard (not HashJoin) for `WHERE group_id = $1`; deny when `$1` unauthorized
- **Plan cache:** Alice/Bob/Alice same SQL, different rows; no folded user id in cached plan
- **DF fixture:** `EXPLAIN` of injected SemiJoin contains `HashJoinExec` / `LeftSemi`
- Membership grant/revoke while connected (fail closed; no historical backfill on grant)
- ON CONFLICT hidden PK; batch WITH CHECK atomicity
- Raw download deny for User/Service
- **CLI e2e:** `cli/kalam-cli-e2e/tests/smoke/security/smoke_test_shared_table_rls.rs` (ACCESS_LEVEL reject, default-deny, CREATE POLICY, plan-cache isolation, ON CONFLICT reject, live grant/revoke fail-closed, raw download deny)

Benchmarks (drive thresholds; do not guess): no-RLS baseline, row-local, PointGuard cache/index, MultiGuard, DF LeftSemi, small vs large build side, auth cache hit/miss, hot vs hot+cold vs parquet-heavy, live fan-out. Cardinalities 1,5,25,100,150,500,1k,10k; tables 100k/1M/10M. Record planning, auth, scan, rows scanned/materialized, bytes, row groups pruned, CPU, allocs, peak mem, P50/P95/P99 vs no RLS.

---

## Out of v1

RESTRICTIVE; arbitrary policy functions; multi-hop graphs; USER/STREAM policies; ENABLE/DISABLE; owner bypass; live historical backfill; raw-file row filtering; automatic index creation; MERGE unless evaluator-complete; custom SQL-path IN/HashSet splitter.

---

## Architectural invariant

```text
PostgreSQL policy SQL
  → one compiler (AuthorizationRelation / RowLocal)
  → one semantic evaluator
  → cheapest strategy (PointGuard | MultiGuard | DF SemiJoin | cached set)
  → SQL / DML / catch-up / live
```

If a path cannot call that evaluator, it must not read or write an RLS-protected shared table.
