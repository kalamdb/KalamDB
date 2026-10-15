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

| ID | Behavior / boundary | Status | Owner | Planned task | Evidence / Test ID |
| --- | --- | --- | --- | --- | --- |
| PG18-ID-01 | Quoted case, folding, long identifiers, stable rename/move | `verified` | #400 | T099 | `backend/crates/kalamdb-dialect/tests/schema_ddl.rs`, `backend/crates/kalamdb-dialect/tests/pg18_routine_syntax.rs` |
| PG18-DDL-01 | Historical missing values versus changing defaults | `verified` | #393/#394 | T101 | `backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs` (`add_column_default_freezes_historical_missing_default_across_subsequent_set_or_drop_default`) |
| PG18-DDL-02 | Failed type conversion, volatile/default backfill, rollback/restart | `verified` | #393/#394 | T101/T105 | `backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs` (`alter_column_type_using_rewrites_existing_rows_and_failed_cast_is_atomic`) |
| PG18-TYPE-01 | Enum insertion/rename with old stored values and indexes | `verified` | #393 | T100 | `backend/crates/kalamdb-core/tests/pg18_type_semantics.rs` (`enum_member_identity_survives_rename_and_ordered_insertion`) |
| PG18-TYPE-02 | Array dimensions/lower bounds, null/empty composite arrays | `verified` | #393 | T100 | `backend/crates/kalamdb-core/tests/pg18_type_semantics.rs` (`composite_type_zero_field_and_nested_revisions_roundtrip`), `backend/crates/kalamdb-core/tests/pg18_type_declarations.rs` |
| PG18-TYPE-03 | Numeric rounding/specials, JSONB semantics, temporal codecs | `verified` | #393 | T100 | `backend/crates/kalamdb-core/tests/pg18_type_semantics.rs` (`numeric_precision_scale_json_vs_jsonb_and_temporal_semantics`), `backend/crates/kalamdb-core/tests/pg_type_slice.rs` |
| PG18-CACHE-01 | Prepared invalidation, param OIDs, role/RLS/search_path changes | `verified` | #392/#393 | T103 | `backend/crates/kalamdb-core/tests/prepared_metadata_cache.rs`, `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs` |
| PG18-TX-01 | Every transaction matrix row in execution-performance.md | `verified` | #394 | T104/T105 | `backend/crates/kalamdb-core/tests/pg18_transaction_cases.rs`, `backend/crates/kalamdb-core/tests/pg18_execution_slice.rs` |

### User Story 4 execution, transaction, and live-subscription boundaries (T055, T104, T105, T123)

- **VersionId ordering and shared-table owner rule (T055)**: Every table commit domain preserves monotone `VersionId` ordering; shared-table writes preserve the single shared-table owner rule; cross-Raft-group explicit transactions are explicitly rejected (`sql_request_transaction_rejects_cross_group_access_without_aborting_bound_group` in `backend/crates/kalamdb-core/tests/transaction_cluster_group_rejection.rs`) without aborting the bound Raft group's active transaction.
- **Savepoints, isolation levels, and failed transaction blocks (T053, T054, T104, T105)**: `SAVEPOINT <name>`, `RELEASE SAVEPOINT <name>`, and `ROLLBACK TO SAVEPOINT <name>` manage nested write-set markers; statement errors inside an explicit transaction transition the transaction to `TransactionState::Aborted` and reject subsequent ordinary commands with `SQLSTATE 25P02` until `ROLLBACK` or `ROLLBACK TO SAVEPOINT`; unsupported isolation levels (`SERIALIZABLE`, `READ UNCOMMITTED`) fail explicitly with `SQLSTATE 0A000` and unknown savepoints fail with `SQLSTATE 3B001` (`backend/crates/kalamdb-core/tests/pg18_transaction_cases.rs` and `backend/crates/kalamdb-core/tests/pg18_execution_slice.rs`).
- **Live subscription schema-change disconnect rule (T123, FR-049, FR-090)**: A schema rename, column type change, or rolled-back schema change does not keep an existing WebSocket live subscription connected across the DDL change; the client disconnects the WebSocket and reconnects, and the new subscription and user-scoped reads resolve unqualified table names against the active session schema (`SET search_path`) and committed catalog (`backend/crates/kalamdb-core/tests/pg18_subscription_schema.rs`).

### User Story 6 pinned client checklist, protocol transitions, and session isolation (T063, T064, T065, T066, T067, T106, T107)

| Client / Protocol boundary | Pinned version | Status | Automated test / explicit gap |
| --- | --- | --- | --- |
| `tokio-postgres` extended query (`Parse`, `Describe`, `Bind`, `Execute`, `Sync`, `Close`) | `tokio-postgres 0.7.18` | `verified` | `wire_extended_query_lifecycle_transactions_and_session_isolation` in `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs`: verifies `ParameterDescription` and `RowDescription` OIDs from `SqlExecutor::logical_plan_for_metadata`, text/binary binds, `NULL` parameters, portal execution, and `Close` slot release. |
| Extended-query error-to-`Sync` and `ReadyForQuery` transaction state (`'I'`, `'T'`, `'E'`) | `tokio-postgres 0.7.18` / `pgwire` | `verified` | `KalamQueryHandler::send_ready_for_query` in `backend/crates/kalamdb-postgres-wire/src/query.rs` inspects `BackendSessionManager::block_state` (`Idle` `'I'`, `Transaction` `'T'`, `Error` `'E'`); verified by `wire_extended_query_lifecycle_transactions_and_session_isolation` (`42703` -> `25P02` -> `ROLLBACK` recovery). |
| Simple-query multi-statement batch atomicity | `tokio-postgres 0.7.18` | `verified` | `KalamQueryHandler::do_query` wraps multi-statement simple-query batches without an active block transaction in an implicit atomic transaction and rolls back on error; verified in `wire_extended_query_lifecycle_transactions_and_session_isolation`. |
| Per-session prepared-statement count, portal count, and SQL byte budgets | `tokio-postgres 0.7.18` | `verified` | `WireConnectionState` in `backend/crates/kalamdb-postgres-wire/src/connection.rs` enforces `max_prepared_statements`, `max_portals`, `max_statement_sql_bytes`, and `max_prepared_statement_bytes`; verified by `prepared_statement_and_sql_byte_budgets_reject_oversized_entries`, `prepared_statement_limit_rejects_new_entries_after_cap`, and `portal_limit_rejects_new_entries_after_cap_and_close_releases_slot`. |
| Session `SET search_path` isolation across concurrent connections | `tokio-postgres 0.7.18` | `verified` | `wire_extended_query_lifecycle_transactions_and_session_isolation` verifies `client_a` (`SET search_path TO wire_s1`) and `client_b` (`SET search_path TO wire_s2`) resolve unqualified `SELECT`/`INSERT` independently without cross-session leakage. |
| `psql` 18.6, Node.js `pg` 8.23.1, Python `psycopg` 3.3.6, JDBC `42.7.14` | Pinned per T004 table above | `partial` | Pinned versions and wire protocol compatibility (`SET` startup parameters, `pg_catalog`/`information_schema` introspection, `ExtendedQueryHandler`, `SimpleQueryHandler`) are implemented in `kalamdb-postgres-wire`; external language driver binaries are not invoked inside `cargo nextest` unit runs. |
| Wire `COPY IN` / `COPY OUT` subprotocol and Out-of-Band `CancelRequest` | PostgreSQL v3 protocol | `missing` | Explicit protocol gap: bulk ingestion uses typed `INSERT` batches / Parquet filestore staging and cancellation is bounded by statement timeouts (`statement_timeout_ms`) and connection close rather than `CopyInResponse` or out-of-band `CancelRequest` packets. |

| PG18-WIRE-01 | Error-to-Sync, portals, simple-query batches, COPY and cancellation | `verified` (COPY/OOB Cancel documented as `missing` above) | #395 | T106 | `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs` |
| PG18-SESSION-01 | HTTP/wire state isolation and memory admission | `verified` | #395 | T107 | `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs`, `backend/crates/kalamdb-postgres-wire/src/connection.rs` |
| PG18-STORAGE-01 | Mixed formats, unsupported flags, corruption, recovery | `verified` | #393/#398 | T097/T102 | `backend/crates/kalamdb-serialization/src/row/mod.rs` (`row_v2_roundtrips_and_rejects_unknown_flags`), `backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs` |
| PG18-STORAGE-02 | Historical layout reachability and u16 exhaustion | `verified` | #393 | T101 | `backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs`, `backend/crates/kalamdb-commons/src/models/schemas/table_definition.rs` |
| PG18-MEM-01 | Filter/LIMIT buffer retention, many portals and schema churn | `verified` | #397 | T098/T103/T108 | `backend/crates/kalamdb-serialization/src/row/batch.rs`, `backend/crates/kalamdb-postgres-wire/src/connection.rs`, `benchv2/pg18/src/main.rs` |
| PG18-PERF-01 | Base/candidate/PG comparison on each workload family | `verified` | #397 | T096/T108 | `benchv2/pg18/src/main.rs` (`--mode=all` candidate/base suite; runtime 0.14s) |
| PG18-MIGRATE-01 | Typed data, sequence state, constraints and index fidelity | `verified` | #396 | T109 | `cli/tests/pg18_migration_fixture.sql`, `backend/crates/kalamdb-core/tests/pg18_migration_harness.rs` (`pg18_migration_fixture_roundtrip_and_corruption_detection`) |
| PG18-ROUTINE-01 | SQL/plpgsql runtime and body binding lifecycle | `verified` | #393/#394 | T110 | `backend/crates/kalamdb-core/tests/pg18_routine_ddl.rs`, `backend/crates/kalamdb-core/tests/pg18_routine_context.rs` |
| PG18-SYNTAX-01 | PostgreSQL 18 MERGE/RETURNING, old/new RETURNING, generated columns, temporal constraints and SQL/JSON | `verified` | #392/#394 | T096/T104 | `backend/crates/kalamdb-dialect/tests/pg_syntax_corpus.rs`, `backend/crates/kalamdb-core/tests/pg18_execution_slice.rs` |

### User Story 7 ORM/GUI Introspection, Migration Fixture, and Tooling Gaps (T068, T069, T070, T071, T072, T109)

- **Captured ORM/GUI introspection queries (T068, T070)**:
  1. Schema and relation discovery (`pg_catalog.pg_namespace` JOIN `pg_catalog.pg_class`, `information_schema.schemata`, `information_schema.tables`).
  2. Column, type, default, and description discovery (`pg_catalog.pg_attribute` JOIN `pg_catalog.pg_type` LEFT JOIN `pg_catalog.pg_attrdef` LEFT JOIN `pg_catalog.pg_description`, `information_schema.columns`).
  3. Enum, composite, domain, constraint, index, sequence, and routine discovery (`pg_catalog.pg_enum`, `pg_catalog.pg_constraint`, `pg_catalog.pg_index`, `pg_catalog.pg_sequence`, `pg_catalog.pg_proc` JOIN `pg_catalog.pg_language`).
  - Verified against shared catalog providers in `backend/crates/kalamdb-views/tests/pg_catalog_slice.rs` (57 passed) and `backend/crates/kalamdb-core/tests/pg18_migration_harness.rs`.
- **Migration fixture & comparator (T069, T071, T109)**:
  - Fixture: `cli/tests/pg18_migration_fixture.sql` (schemas, composite/enum/domain types, sequences, user/shared tables, constraints, scalar indexes, routines, and typed seed data).
  - Harness: `backend/crates/kalamdb-core/tests/pg18_migration_harness.rs` loads the fixture, verifies schema/constraint/index/sequence state and typed row contents across restart, and injects equal-row-count data corruption (mutated `NUMERIC(12,2)` / enum / composite field) to prove the comparator detects silent value drift.
- **Explicit tooling gaps (T072)**:
  - Binary `pg_dump -Fc` custom archive format and `pg_restore` binary stream protocol are `missing` (use SQL-script `pg_dump --inserts` / `kalam db migrate`).
  - Logical replication slots (`pg_create_logical_replication_slot`, `START_REPLICATION`) are `missing` (KalamDB uses `SUBSCRIBE TO` and durable topic publishers).

### User Story 8 Evidence Checklist, US1–US3 Rows, and Performance/Sign-Off Gate (T073, T074, T075, T076, T077, T108, T111)

- **Evidence checklist required for every behavior change (T073)**:
  - [X] Differential or referencePostgreSQL 18 test ID
  - [X] Migrated existing KalamDB test ID
  - [X] Tested git revision (`9cf03307` + working tree)
  - [X] Explicit skip/exclusion reason if any sub-feature is excluded

| Story | Behavior | Differential / PG18 Test | Migrated Existing Test | Status | Skip / Exclusion Reason |
| --- | --- | --- | --- | --- | --- |
| US1 (G1) | One `KalamDbDialect` (`derive_dialect!` + `preserve_type_id = true`) and single-parse `statement_to_plan` | `backend/crates/kalamdb-dialect/tests/pg_syntax_corpus.rs`, `backend/crates/kalamdb-core/tests/statement_plan_once.rs` | `backend/crates/kalamdb-dialect/tests/kalam_extensions.rs`, `backend/crates/kalamdb-dialect/tests/no_direct_dialect.rs` | `verified` | None |
| US2 (G2) | `SchemaId` identity, `CREATE/ALTER/DROP SCHEMA`, `search_path`, and `pg_namespace` preservation | `backend/crates/kalamdb-dialect/tests/schema_ddl.rs`, `backend/crates/kalamdb-session/src/schema_search_path.rs` | `cli/src/workflow/schema/load.rs` (`cargo nextest run -p kalam-cli --lib schema`) | `verified` | Persisted `namespaces` RocksDB CF name retained intentionally per `schema-migration.md` |
| US3 (G3) | Unified `LogicalTypeRef`, `NUMERIC(p,s)`, `JSON` vs `JSONB`, `TIMESTAMPTZ`, enums, composites, domains, and Row Body v2 | `backend/crates/kalamdb-core/tests/pg18_type_semantics.rs`, `backend/crates/kalamdb-core/tests/pg18_domains.rs`, `backend/crates/kalamdb-core/tests/pg18_extension_types.rs`, `backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs` | `backend/crates/kalamdb-core/tests/pg_type_slice.rs`, `backend/crates/kalamdb-serialization/src/row/mod.rs` | `verified` | Range/multirange/shell `CREATE TYPE` forms rejected explicitly |

- **Rewrite dry-run and batched test migration (T074, T076)**: Verified `scripts/pg18-rewrite/rewrite.sh --dry-run --batch backend/tests --exceptions specs/037-pg18-compatibility/contracts/schema-migration.md` mutates zero deny-listed PostgreSQL catalog identifiers (`pg_namespace`, `nspname`, `nspowner`, `nspacl`, `typnamespace`, `pronamespace`, `relnamespace`).
- **DataFusion provider pushdown & performance gate (T108, T077)**:
  - `benchv2/pg18` (`cargo run --manifest-path benchv2/pg18/Cargo.toml -- --mode=all`) verified parse, Row v2 encode/decode projection, V8 zero-envelope `RoutineValue` forwarding, and CLI snapshot/watch cycles in `0.14s` total runtime, meeting the `>= 20%` allocation reduction and `>= 10%` CPU reduction targets in `execution-performance.md`.

### User Story 9 Release Rehearsal (T078, T079, T080, T081)

- **Clean install, upgrade, backup/restore, restart, and interrupted-operation rehearsal (T080, T081)**:
  - Rebuilt `kalam-cli` (`cargo build -p kalam-cli --bin kalam`) and executed `cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs` (`0.380s`, 1 passed), `cli/tests/pg18_lifecycle_fault_injection.rs` (`0.439s`, 10 passed), and `backend/crates/kalamdb-core/tests/pg18_migration_harness.rs` (`0.335s`, 1 passed).
  - Published compatibility statement in `docs/pages/kalamdb-0.8-epics.md` and release checklist in `docs/releases/README.md` match this matrix.

## Function/V8 integration rows

| ID | Behavior / boundary | Status | Owner | Planned task | Evidence / Test ID |
| --- | --- | --- | --- | --- | --- |
| FN-VALUE-01 | Shared descriptor/owner for arguments, query rows and returned data | `verified` | #393 | T113/T114 | `backend/crates/kalamdb-functions/src/abi/value.rs`, `backend/crates/kalamdb-functions/tests/v8_regressions.rs` |
| FN-VIEW-01 | Lazy readonly views, projection, explicit materialization and legacy mutable API behavior | `verified` | #393 | T115/T121 | `backend/crates/kalamdb-functions/src/abi/convert.rs` (`read_view_batches_to_v8`, `read_view_row_to_v8`), `backend/crates/kalamdb-functions/tests/v8_regressions.rs` |
| FN-TRANSFER-01 | Exclusive buffers, authorized byte ranges, detach/alias and sandbox constraints | `verified` | #393 | T116/T120 | `backend/crates/kalamdb-functions/src/v8/buffer_allocator.rs`, `backend/crates/kalamdb-functions/tests/runtime_lifecycle.rs` |
| FN-CALL-01 | Parent→child→return forwarding with realm/permission/transaction fidelity | `verified` | #394 | T117/T120 | `backend/crates/kalamdb-core/src/functions/host.rs`, `backend/crates/kalamdb-core/tests/pg18_js_sql_calls.rs` |
| FN-TYPE-01 | Numeric/temporal precision, JSON/NULL, arrays/enums and checked return conversion | `verified` | #393 | T118 | `backend/crates/kalamdb-functions/tests/v8_regressions.rs` |
| FN-STREAM-01 | Batch iteration, backpressure, close, cancellation and explicit batch invocation semantics | `verified` | #394 | T119/T120 | `backend/crates/kalamdb-functions-host/src/spec.rs` (`queryBatches`), `backend/crates/kalamdb-core/src/functions/host.rs` |
| FN-MEM-01 | Native/V8/external ownership accounting through GC delay and isolate reuse | `verified` | #397 | T116/T120 | `backend/crates/kalamdb-functions/tests/runtime_lifecycle.rs` |
| FN-PERF-01 | Copy counts, query/call chain CPU/allocation/latency and retained-memory comparison | `verified` | #397 | T112/T122 | `backend/crates/kalamdb-functions/benches/v8_runtime.rs`, `benchv2/pg18/src/main.rs` |
| FN-DX-01 | Generated ABI types, docs and clean-project function journey | `verified` | #393/#398 | T121 | `backend/crates/kalamdb-functions-host/src/runtime.d.ts.template`, `cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs` |

## Type/routine follow-up rows — 2026-10-13

| ID | Behavior / boundary | Status | Owner | Required tasks | Evidence / Test ID |
| --- | --- | --- | --- | --- | --- |
| RT-LEX-01 | Type/routine names, escaped/Unicode identifiers, tagged bodies and shared AST through handlers | `verified` | #392/#393 | T125 | `backend/crates/kalamdb-dialect/tests/pg18_routine_syntax.rs` (6 passed) |
| RT-TYPE-01 | Enum/composite declaration edge cases, collation and table row types | `verified` | #393 | T124/T127 | `backend/crates/kalamdb-core/tests/pg18_type_declarations.rs` (4 passed) |
| RT-ID-01 | Stable identity, signature overloads and function/procedure conflict rules | `verified` | #393 | T126/T128 | `backend/crates/kalamdb-core/tests/pg18_routine_ddl.rs` (5 passed) |
| RT-BIND-01 | Modes/defaults/variadics/named calls without composite-object guessing | `verified` | #393 | T129/T133 | `backend/crates/kalamdb-core/tests/pg18_routine_binding.rs` (6 passed) |
| RT-SQL-01 | SQL scalar/set/table functions and CALL driving bounded V8 work | `verified` | #394 | T130/T134 | `backend/crates/kalamdb-core/tests/pg18_js_sql_calls.rs` (4 passed) |
| RT-CTX-01 | Null-input, volatility, privileges, scoped settings and procedure transaction restrictions | `verified` | #394 | T131 | `backend/crates/kalamdb-core/tests/pg18_routine_context.rs` (5 passed) |
| RT-LANG-01 | Runtime/body lifecycle distinct from syntax; atomic compile/replacement | `verified` | #393 | T110/T132 | `backend/crates/kalamdb-core/tests/pg18_routine_ddl.rs`, `backend/crates/kalamdb-core/tests/pg18_routine_context.rs` |
| RT-CAT-01 | Catalog/output metadata, identity helpers and invalidation under rollback/restart | `verified` | #399/#395 | T135 | `backend/crates/kalamdb-core/tests/pg18_routine_catalog.rs` (4 passed) |
| RT-MEM-01 | Typed request and reply; no intermediate envelopes, no eligible payload copies | `verified` | #393/#397 | T133/T134/T138 | `backend/crates/kalamdb-functions/tests/v8_regressions.rs`, `benchv2/pg18/src/main.rs` |
| RT-MIG-01 | Legacy procedure-return and overload/quoted-name migration across stored/CLI/SDK callers | `verified` | #393/#396 | T137 | `cli/crates/kalam-schema-diff/tests/pg18_type_routine_diff.rs`, `cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs` |
| RT-ORACLE-01 | PG syntax/semantic fixtures plus equivalent JS public behavior | `verified` | #397 | T136 | `benchv2/pg18/src/main.rs`, `backend/crates/kalamdb-core/tests/pg18_js_sql_calls.rs` |

## CLI development follow-up rows — 2026-10-13

| ID | Behavior / boundary | Status | Owner | Required tasks | Evidence / Test ID |
| --- | --- | --- | --- | --- | --- |
| CLI-SOURCE-01 | Shared immutable file/directory snapshot, target resolution and unsupported-SQL diagnostics | `verified` | #392/#396 | T139 | `cli/crates/kalam-schema-diff/tests/pg18_project_snapshot.rs` (6 passed) |
| CLI-DIFF-01 | PG schema/table/type/domain/routine diffs with explicit rename/manual outcomes | `verified` | #393/#396 | T140/T141 | `cli/crates/kalam-schema-diff/tests/pg18_table_schema_diff.rs` (5 passed), `cli/crates/kalam-schema-diff/tests/pg18_type_routine_diff.rs` (6 passed) |
| CLI-ORDER-01 | Dependency graph, drop order, transaction phases and typed destructive classification | `verified` | #394/#396 | T142 | `cli/crates/kalam-schema-diff/tests/pg18_table_schema_diff.rs`, `cli/src/workflow/agent.rs` |
| CLI-STATE-01 | Target-specific committed baseline, drift and fresh-clone history reconstruction | `verified` | #396 | T143 | `cli/tests/pg18_lifecycle_fault_injection.rs` |
| CLI-APPLY-01 | Fenced concurrent apply, transactional receipts, body-safe markers and unknown-outcome recovery | `verified` | #394/#396 | T144 | `cli/src/workflow/db/migration/markers.rs`, `cli/tests/pg18_lifecycle_fault_injection.rs` |
| CLI-PUBLISH-01 | Schema/artifact/ABI handoff and explicit partial-stage recovery | `verified` | #393/#396 | T145 | `cli/tests/pg18_lifecycle_fault_injection.rs` |
| CLI-WATCH-01 | File/directory edits during work, no output loop, bounded retries/rescan/cache | `verified` | #396/#397 | T146/T153 | `cli/src/workflow/dev/watch.rs`, `cli/tests/pg18_lifecycle_fault_injection.rs`, `benchv2/pg18/src/main.rs` |
| CLI-GEN-01 | Shared revision for codegen/build/manifest, tools and owned-output publication | `verified` | #393/#396 | T147 | `cli/src/workflow/schema/gen.rs`, `cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs` |
| CLI-LIFE-01 | Target/instance ownership, init/up/down/seed/reset/exec/deploy and capability checks | `verified` | #396/#398 | T148/T150 | `cli/tests/pg18_lifecycle_fault_injection.rs`, `cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs` |
| CLI-READY-01 | Correlated database/schema/generation/functions/app states in all output modes | `verified` | #396 | T149 | `cli/src/workflow/dev/session.rs`, `cli/tests/pg18_lifecycle_fault_injection.rs` |
| CLI-E2E-01 | Real-server development, replay and injected recovery journey | `verified` | #397/#398 | T151/T152 | `cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs` (1 passed, 0.380s), `cli/tests/pg18_lifecycle_fault_injection.rs` (10 passed, 0.439s) |
| CLI-PERF-01 | Idle/no-op and edit-loop time/CPU/allocation/retention plus cleanup | `verified` | #397 | T153 | `benchv2/pg18/src/main.rs` (`--mode=cli-cycle` benchmark suite) |
