# Quickstart & Validation Guide: Unified Table Processing and Atomic Transaction Optimization

**Feature**: `039-unified-table-transactions`  
**Branch**: `feat/pg-dialect` (PR [#402](https://github.com/kalamdb/KalamDB/pull/402))  
**Build Profile**: Default `dev` profile for all local checks, tests, and baseline/candidate A/B comparisons (per `AGENTS.md`).

---

## 1. Fast Batch Compile Check

After completing a multi-file edit batch, run a single workspace/crate check and capture output:

```bash
cd backend
cargo check -p kalamdb-commons -p kalamdb-datafusion-sources -p kalamdb-store -p kalamdb-tables -p kalamdb-flush -p kalamdb-publisher -p kalamdb-transactions -p kalamdb-plan-cache -p kalamdb-transfer -p kalamdb-core -p kalamdb-jobs -p kalamdb-pg-bridge -p kalamdb-dba -p kalamdb-postgres-wire > ../batch_compile_output.txt 2>&1
```

---

## 2. Targeted Responsibility & Parity Suites (Narrow-to-Broad Gates)

### Gate 1 — Responsibility Unit & Crate Boundary Contract Tests (SC-013, SC-014)
Verify each separated crate/module responsibility independently:

```bash
cd backend
cargo nextest run -p kalamdb-datafusion-sources
cargo nextest run -p kalamdb-store
cargo nextest run -p kalamdb-transactions
cargo nextest run -p kalamdb-tables
cargo nextest run -p kalamdb-publisher
cargo nextest run -p kalamdb-plan-cache
cargo nextest run -p kalamdb-transfer
```

### Gate 2 — Unified DataFusion `KalamTableProvider` & Fast-Path Parity (SC-005, SC-013)

Record the SC-015 system-catalog baseline before editing `kalamdb-system` or `kalamdb-views`, then compare after:

- Direct primary-key and secondary-index lookups for users, jobs, namespaces, and types (these must stay on `IndexedEntityStore`, not DataFusion).
- One `pg_catalog` + `information_schema` introspection query used by wire clients (namespace, table, column).

Five matched `dev`-profile repeats. Median and p99 must be ≤ baseline. Report each runtime in seconds. Any regression blocks the change. The 5% user/shared allowance does not apply.

```bash
cd backend
cargo nextest run -p kalamdb-system
cargo nextest run -p kalamdb-views
cargo nextest run -p kalamdb-core pg_catalog
```

### Gate 2b — User/Shared/Stream Provider Parity (SC-005, SC-013)
Verify that `User`, `Shared`, and `Stream` tables execute through `KalamTableProvider`, and that `User` and `Shared` both pass `columnar_hot_scan`, `sum_hot_covering_integers`, and `plan_literal_equality_updates`:

```bash
cd backend
cargo nextest run -p kalamdb-tables --test provider_source_models
cargo nextest run -p kalamdb-tables --test stream_provider_lightweight_scan
cargo nextest run -p kalamdb-core covering_aggregate
cargo nextest run -p kalamdb-core equality_update
```

### Gate 3 — 25,000-Row Atomic Commit, Snapshot Safety, and Commit Folding (SC-001, SC-002, SC-003, SC-004)
Run the parameterized User + Shared + Stream parity and 25k atomic transaction tests in `kalamdb-core` and `kalamdb-tables`:

```bash
cd backend
cargo nextest run -p kalamdb-core transaction
cargo nextest run -p kalamdb-core applier
```

Key assertions verified by this gate:
- `RecordingBackend::batch_calls()` increments by **1** for a 25,000-row single-table or multi-table transaction in one commit domain (and **0** for an empty / folded no-op transaction).
- Injecting a constraint/uniqueness failure at row 25,000 leaves **0** committed rows, **0** staged vectors, **0** deleted `FileRef`s, **0** manifest pending-write changes, and **0** emitted notifications.
- A concurrent reader pinned to an earlier `snapshot_commit_seq` continues to read hot-only rows across updates, deletes, and subsequent cold flushes.
- All 9 commit-fold transitions and primary-key renames produce identical SQL results and statement row counts for both `User` and `Shared` tables.

### Gate 4 — Columnar Dirty-Transaction Overlay Benchmark & Parity (SC-010)
Verify 100-read and 1,000-read workloads inside a dirty transaction (with predicate movement, partial updates, explicit nulls, and savepoint rollback):

```bash
cd backend
cargo nextest run -p kalamdb-transactions overlay
```

### Gate 5 — Unified Versioned Flush, Lock Decoupling, Compaction, and Bounded Transfers (SC-006, SC-007)
Verify that `VersionedTableFlushJob` flushes both User and Shared tables, `mark_pending_write` does not block on a delayed cold flush, scope locks reclaim to steady state after churn, and exports/imports stream within memory budgets:

```bash
cd backend
cargo nextest run -p kalamdb-flush
cargo nextest run -p kalamdb-jobs
```

### Gate 6 — Contextual Plan Cache, Singleflight Misses, and AST Semantics (SC-008)
Verify `SELECT *` visibility isolation between HTTP SQL and PGWire in both warmup orders, concurrent cold-miss singleflight deduplication, and AST-based `RETURNING` description:

```bash
cd backend
cargo nextest run -p kalamdb-plan-cache
cargo nextest run -p kalamdb-postgres-wire
cargo nextest run -p kalamdb-core sql_executor
```

### Gate 7 — Row V2 Durable Compatibility Barrier & Format Coexistence (SC-011)
Verify mixed V1/V2 row decoding across schema versions, rejection of V2 writer activation when an eligible replica is incompatible, and safe writer disable across restart:

```bash
cd backend
cargo nextest run -p kalamdb-serialization
```

---

## 3. Consolidation Code-Reduction & Zero-Downcast Verification (FR-038–FR-046, SC-005, SC-014)

Verify that:
1. Superseded duplicate provider, store, flush, and applier implementations are deleted and net maintained production lines across the consolidation scope decreased.
2. Zero `.downcast_ref::<UserTableProvider | SharedTableProvider | StreamTableProvider>()` calls remain in production code.
3. `kalamdb-transfer` does not depend on `kalamdb-core` and `kalamdb-publisher` does not depend on `kalamdb-tables`.

```bash
# 1. Check zero concrete provider downcasts in production crates
rg 'downcast_ref::<(User|Shared|Stream)TableProvider>' backend/crates/

# 2. Compute production LOC across the consolidation scope
python3 - <<'PY'
import pathlib

roots = [
    "backend/crates/kalamdb-system/src/providers",
    "backend/crates/kalamdb-system/src/macros.rs",
    "backend/crates/kalamdb-tables/src",
    "backend/crates/kalamdb-flush/src/flush",
    "backend/crates/kalamdb-core/src/applier/executor/dml.rs",
    "backend/crates/kalamdb-jobs/src/executors/flush.rs",
]

total_prod_lines = 0
for r in roots:
    p = pathlib.Path(r)
    files = [p] if p.is_file() else sorted(p.rglob("*.rs"))
    for f in files:
        if not f.exists():
            continue
        lines = f.read_text().splitlines()
        prod_lines = 0
        for line in lines:
            if line.strip().startswith("#[cfg(test)]"):
                break
            prod_lines += 1
        total_prod_lines += prod_lines
        print(f"{prod_lines:6d}  {f}")

print(f"TOTAL PRODUCTION LINES IN CONSOLIDATION SCOPE: {total_prod_lines}")
PY
```

---

## 4. End-to-End CLI Smoke Gate (Mandatory Before Commit)

Start the backend server in default dev mode, then run the CLI smoke suite:

```bash
# Terminal 1: Start server
cd backend && cargo run --bin kalamdb-server

# Terminal 2: Run CLI smoke tests
cd cli
cargo build -p kalam-cli --bin kalam
KALAMDB_SERVER_URL="http://localhost:3000" KALAMDB_ROOT_PASSWORD="mypass" cargo nextest run -p kalam-cli-e2e --test e2e smoke
```

---

## 5. Performance Evidence Reporting Protocol (FR-036, SC-009)

For every benchmark or perf test run:
1. Run at least **5 matched repeats** under identical hardware, data, concurrency, durability, and default `dev` profile settings.
2. Record each run's elapsed time **in seconds** (`f64`), plus p50/p95/p99 latency, `StorageBackend::batch` call count, PK target loads, and retained memory bytes.
3. Verify median improvement exceeds baseline run-to-run spread and unaffected p95/p99 metrics do not regress by more than 5%.
