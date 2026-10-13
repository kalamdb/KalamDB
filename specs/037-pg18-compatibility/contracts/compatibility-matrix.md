# Contract: Compatibility Matrix

The matrix is the release scope. It is not a claim that today's server matches PostgreSQL 18.

## Item

Each item has the fields in [data-model.md](../data-model.md). Status is only `not_assessed`, `missing`, `partial`, or `verified`.

`verified` requires an automated test, the tested revision, and the PostgreSQL 18 and client versions. A closed GitHub issue is not `verified`.

An exclusion is a product decision written on the item and repeated in the release notes. The item does not become `verified`.

## Surfaces that must be inventoried

- SQL commands, including identifiers, literals, casts, parameters, DDL, DML, joins, subqueries, CTEs, windows, conflict handling, and returning clauses
- Types, aliases, modifiers, and categories, including the families in spec FR-063
- Functions, operators, aggregates, and routines
- Transactions, savepoints, isolation, locking, and failure
- Protocol messages, including simple and extended query, prepared statements, cancellation, and COPY where required
- `pg_catalog`, `information_schema`, and the helper functions listed in spec FR-095 and FR-096
- Client workflows for pinned `psql` and the selected Node.js, Python, Rust, and JDBC drivers

Extensions, procedural languages other than the supported set, backup tools, and replication are their own items so they cannot hide inside a broad row.

## Baseline

PostgreSQL 18 SQL, data types, catalogs, information schema, and the frontend/backend protocol. Kalam file values, embedding values, and JavaScript or TypeScript routines are extension items, marked distinct from PostgreSQL behavior.

### Pinned server and clients

Pinned on 2026-10-11 for T004. These are the versions differential and client checks must use. A newer local binary is not a substitute.

| Role | Pin | Source |
| --- | --- | --- |
| PostgreSQL server | 18.6, released 2026-08-13 | Official image `postgres:18.6`. 18.5 was not released. |
| psql | 18.6, the client from that image | The machine used for this pin has psql 18.4. Do not use that binary as the oracle client. |
| Node.js driver | `pg` 8.23.1, published 2026-09-30 | npm `pg` |
| Python driver | psycopg 3.3.6, released 2026-09-18 | PyPI `psycopg` |
| Rust driver | `tokio-postgres` 0.7.18 | Workspace `Cargo.toml`. Workspace `rust-version` is 1.94. |
| JDBC driver | `org.postgresql:postgresql` 42.7.14, released 2026-10-07 | Maven Central |

### T096 workload parameters, budgets, and baseline

Recorded 2026-10-12 from `cargo run --manifest-path benchv2/pg18/Cargo.toml -- --mode=baseline`. The binary is `kalam-pg18-harness`. This is a dev-profile sqlparser parse on one machine. It is not a server workload and not a PostgreSQL comparison. No candidate build exists, so no candidate/base or candidate/PG ratio is reported.

| Parameter | Recorded value |
| --- | --- |
| Revision | `be5efdd2`, plus the uncommitted harness |
| Compiler | `rustc 1.100.0-nightly (67854e511 2026-08-15)` |
| Machine | Apple M5 Pro, 15 cores, 24 GiB, arm64 |
| Build profile | dev (`cargo run` default, unoptimized) |
| Parser | sqlparser 0.62.0, `PostgreSqlDialect` |
| Corpus | 10 statements, 483 bytes. Selects, casts, parameters, arrays, JSON operators, dollar quotes, nested comments, insert, update, and delete. |
| Samples | 5, after 1 warmup |
| Hot loop | 1000 parses of `SELECT 1` |
| Concurrency measured | 1 |
| Concurrency required | 1, 8, 32, and 128 |
| Concurrency 8, 32, 128 | `not_measured`. No server run was executed. |
| PostgreSQL 18.6 | `not_measured`. `KALAM_PG18_URL` was unset. Local psql 18.4 is not the oracle client. |
| Point read, scan, join, DML, flush, catalog churn, narrow projection | `not_measured` |

Cold scope is one parse of each corpus statement. Hot scope is the 1000-parse loop.

| Sample | Cold seconds | Hot seconds |
| --- | --- | --- |
| 1 | 0.000141500 | 0.003265625 |
| 2 | 0.000133458 | 0.003326958 |
| 3 | 0.000148750 | 0.003232417 |
| 4 | 0.000131250 | 0.003279375 |
| 5 | 0.000137833 | 0.003224416 |
| Median | 0.000137833 | 0.003265625 |

Plan-cache admission budgets now in `kalamdb-plan-cache`, fixed before user-story behavior changes:

| Budget | Value |
| --- | --- |
| Entry count | 1000 |
| Idle TTL | 900 seconds |
| Estimated retained bytes | 64 MiB |
| Per-entry estimate | 1 MiB |

An entry over the per-entry or total estimate is rejected and not stored. These are admission limits. They are not measured process RSS. Portal, session, and slow-reader budgets stay with later tasks.

Acceptance targets below are gates from `execution-performance.md`. They are not results from this run.

- Replaced SELECT and flush materialization paths: at least 20% lower allocation bytes per row, and at least 10% lower CPU per row or median runtime.
- No unexplained regression above 5% in median or p95, or above 10% in p99 or peak/retained memory.
- A weighted encoded-size increase above 5% blocks the format switch unless an explicit workload decision accepts it.

### T005 correctness baseline

Run on 2026-10-11 from `backend/` on revision `be5efdd2` plus the uncommitted `derive-dialect` feature. That feature is unused, so it does not change parse results. These are correctness runs. They are not performance evidence.

| Command | Result |
| --- | --- |
| `cargo nextest run -p kalamdb-dialect` | 623 passed, 0 failed, 1 skipped. Test runtime 1.238s. |
| `cargo nextest run -p kalamdb-postgres-wire` | 33 passed, 0 failed, 0 skipped. Test runtime 0.076s. |

The skipped dialect test is `test_validation_invalid_column_names` in `backend/crates/kalamdb-dialect/tests/test_create_table_constraints.rs`. It is `#[ignore]` and expects a backtick identifier to be rejected. No current correctness failure was recorded in either crate.

## Ownership

#397 owns the matrix and the harness. Each other tracker updates the rows it implements. #399 owns catalog rows. #396 consumes them and owns tooling and migration rows.

## Mandatory regression rows from second review

These are test specifications, all currently `not_assessed` by this review. A proposed test file is not passing evidence. T096 records the pinned versions, revision, actual test IDs and measurements before updating statuses.

| ID | Behavior / boundary | Owner | Planned task |
| --- | --- | --- | --- |
| PG18-ID-01 | Quoted case, folding, long identifiers, stable rename/move | #400 | T099 |
| PG18-DDL-01 | Historical missing values versus changing defaults | #393/#394 | T101 |
| PG18-DDL-02 | Failed type conversion, volatile/default backfill, rollback/restart | #393/#394 | T101/T105 |
| PG18-TYPE-01 | Enum insertion/rename with old stored values and indexes | #393 | T100 |
| PG18-TYPE-02 | Array dimensions/lower bounds, null/empty composite arrays | #393 | T100 |
| PG18-TYPE-03 | Numeric rounding/specials, JSONB semantics, temporal codecs | #393 | T100 |
| PG18-CACHE-01 | Prepared invalidation, param OIDs, role/RLS/search_path changes | #392/#393 | T103 |
| PG18-TX-01 | Every transaction matrix row in execution-performance.md | #394 | T104/T105 |
| PG18-WIRE-01 | Error-to-Sync, portals, simple-query batches, COPY and cancellation | #395 | T106 |
| PG18-SESSION-01 | HTTP/wire state isolation and memory admission | #395 | T107 |
| PG18-STORAGE-01 | Mixed formats, unsupported flags, corruption, recovery | #393/#398 | T097/T102 |
| PG18-STORAGE-02 | Historical layout reachability and u16 exhaustion | #393 | T101 |
| PG18-MEM-01 | Filter/LIMIT buffer retention, many portals and schema churn | #397 | T098/T103/T108 |
| PG18-PERF-01 | Base/candidate/PG comparison on each workload family | #397 | T096/T108 |
| PG18-MIGRATE-01 | Typed data, sequence state, constraints and index fidelity | #396 | T109 |
| PG18-ROUTINE-01 | SQL/plpgsql runtime and body binding lifecycle | #393/#394 | T110 |
| PG18-SYNTAX-01 | PostgreSQL 18 MERGE/RETURNING, old/new RETURNING, generated columns, temporal constraints and SQL/JSON | #392/#394 | T096/T104 |

Each group expands into independent matrix items with positive, negative and lifecycle tests before it may be called verified; a single passing case cannot verify an entire family. Syntax-only, runtime, storage, protocol and client coverage are recorded separately. Required items without an approved exclusion remain release blockers.

Performance evidence also records baseline/candidate commits, exact workload/configuration, repetitions, time in seconds, throughput, latency quantiles, CPU, allocation bytes/counts, peak and retained memory, encoded size, and the preregistered budgets in [execution-performance.md](execution-performance.md). PG ratios identify mismatched configurations and are not aggregate compatibility claims.

## Function/V8 integration rows

All rows are `not_assessed` by this planning extension; existing implementations need the listed acceptance evidence. JS/TS view/transfer APIs are Kalam extensions, not PostgreSQL compatibility claims.

| ID | Behavior / boundary | Owner | Planned task |
| --- | --- | --- | --- |
| FN-VALUE-01 | Shared descriptor/owner for arguments, query rows and returned data | #393 | T113/T114 |
| FN-VIEW-01 | Lazy readonly views, projection, explicit materialization and legacy mutable API behavior | #393 | T115/T121 |
| FN-TRANSFER-01 | Exclusive buffers, authorized byte ranges, detach/alias and sandbox constraints | #393 | T116/T120 |
| FN-CALL-01 | Parent→child→return forwarding with realm/permission/transaction fidelity | #394 | T117/T120 |
| FN-TYPE-01 | Numeric/temporal precision, JSON/NULL, arrays/enums and checked return conversion | #393 | T118 |
| FN-STREAM-01 | Batch iteration, backpressure, close, cancellation and explicit batch invocation semantics | #394 | T119/T120 |
| FN-MEM-01 | Native/V8/external ownership accounting through GC delay and isolate reuse | #397 | T116/T120 |
| FN-PERF-01 | Copy counts, query/call chain CPU/allocation/latency and retained-memory comparison | #397 | T112/T122 |
| FN-DX-01 | Generated ABI types, docs and clean-project function journey | #393/#398 | T121 |
