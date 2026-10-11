# Quickstart: PostgreSQL-Only SQL Language Validation

**Status**: Required implementation validation, not tests already run. New test files named below are created by tasks.md. Run from the repository root unless a command changes directory.

## 1. Capture baseline before changing parser behavior

Run existing focused tests under the default dev profile:

```bash
cargo nextest run -p kalamdb-dialect
cargo nextest run -p kalamdb-core --test test_datafusion55_sql_features --test prepared_metadata_cache
```

Setup tasks add a corpus directory `backend/crates/kalamdb-dialect/tests/fixtures/postgresql/`, an inventory and `validation.md` in this feature. Freeze currently working GUI/JDBC SQL, schemas, visible rows and column metadata. Record client/server/dependency versions and which transports were exercised. Existing failures are recorded, not relabeled as passes. Required feature cases that currently fail are regression targets.

Create and run `backend/crates/kalamdb-core/tests/postgresql_pipeline_perf.rs` against the pre-change pipeline, then the revised one. Use identical seeded catalogs, SQL corpus, dev profile and hardware: at least 100 warmups and 1,000 measured iterations per case, three repetitions, median per case. Separate cold parse/classify/authorize/plan from warm prepared execution. Record total/per-case runtime in seconds, allocations/bytes, cache hits and parse counts. Both comparable medians must meet SC-009's +10% limit. Parse-only timing is diagnostic, not compared to an old classification-only fast path.

## 2. Standard SQL, table kinds and sessions (US1/US2)

Tests create and clean their own schemas/users/procedures with existing harness helpers. At minimum execute:

```sql
CREATE SCHEMA IF NOT EXISTS dialect_app;
CREATE TABLE dialect_app.items (id BIGINT PRIMARY KEY, name TEXT NOT NULL);
INSERT INTO dialect_app.items VALUES (1, 'a');
SET search_path TO dialect_app;
SELECT name FROM items;
CREATE TABLE dialect_app.inbox (id BIGINT PRIMARY KEY) WITH (TYPE = 'USER');
```

Both SELECT resolution and catalog kinds must match expectations. Separate negative requests test CREATE/DROP USER TABLE, duplicate/invalid TYPE and unsupported constraints without partial mutations. Create a supported-runtime procedure before testing CALL/GRANT. Test policies and indexes on appropriate table kinds. Identity DDL must never create a table.

Session fixtures create two schemas with duplicate table names and verify ordered resolution, `"$user"`, explicit/implicit pg_catalog, quoted names, SET LOCAL rollback/commit cleanup, RESET and stateful HTTP/wire parity. Stateless HTTP tests keep dependent statements in one request. Cover unsupported transaction options and role/tenant denials.

```bash
cargo nextest run -p kalamdb-dialect --test postgresql_standard --test postgresql_table_kinds
cargo nextest run -p kalamdb-core --test postgresql_sessions --test postgresql_ddl
```

## 3. One execution language and prepared reuse (US3)

```sql
SELECT ARRAY[1, 2, 3];
SELECT * FROM UNNEST(ARRAY[1, 2, 3]) AS t(value);
SELECT array_transform(ARRAY[1, 2, 3], x -> x * 10);
SELECT '{"k": 1}'::jsonb -> 'k';
SET datafusion.sql_parser.dialect = 'duckdb';
```

Run independently. Arrays/UNNEST execute where required by the matrix. Lambda execution and dialect override fail with documented categories. JSON parses as PostgreSQL; execute if supported, otherwise assert 0A000 rather than a lambda/syntax error. Test `->>`, casts, CTEs, joins, windows, ON CONFLICT and RETURNING using the matrix's expected outcomes. Retry/replan after catalog changes reuses the parsed AST. Wire Parse/Bind/Describe/Execute tests assert parameter/result types and principal/search-path isolation.

Unchanged GUI fixtures must retain rows/metadata; do not replace queries to make the suite pass. Parser counters assert one backend parse per submitted statement and no extra parse at bind/metadata/cache-hit/retry stages.

```bash
cargo nextest run -p kalamdb-dialect --test postgresql_corpus
cargo nextest run -p kalamdb-core --test postgresql_pipeline --test postgresql_authorization
cargo nextest run -p kalamdb-postgres-wire --test wire_extended_query --test wire_transactions --test wire_smoke
```

Wire suites use their repository harness prerequisites; integration suites requiring an external listener must run against the configured test server, not be skipped silently.

## 4. Extensions and stored SQL (US4)

Create the mixed-case object before flushing it. Cover every extension family with valid, malformed, comment/quote and denied-role cases. Subscription predicates use shared expressions. Execute stored views, schedules and procedure host SQL before/after definition changes. Non-SQL procedure bodies remain intact, including internal semicolons/dollar-like strings.

```bash
cargo nextest run -p kalamdb-dialect --test postgresql_extensions
cargo nextest run -p kalamdb-core --test postgresql_stored_sql
cargo nextest run -p kalamdb-api --test postgresql_subscriptions
```

## 5. Producers, aliases and upgrade (US5)

CLI split/generation, UI completion, SDK handwritten generators and bridge-emitted SQL all produce the canonical corpus. Use existing package scripts for SDK/UI tests; regenerate rather than edit generated outputs. Run PostgreSQL bridge integration through `pg/test.sh` with its prerequisites and record results. Typed FDW calls are tested for equivalent behavior, not artificially converted into SQL.

Before server replacement, export SQL-bearing definitions using the existing catalog/SQL APIs as JSON records containing object kind, ID, version and SQL. The implementation supplies this read-only offline runner:

```bash
cargo run -p kalamdb-core --example check_postgresql_sql_upgrade < /tmp/definitions.json
```

It exits nonzero on incompatible definitions, reports safe object identities, and makes no network or catalog writes. Startup runs the same check before catalog mutations. The read-only preflight fixture detects a legacy stored definition, reports object identity without leaking its body/secrets, and makes no catalog changes. Apply the documented replacement through existing APIs and prove execution after restart. Repeat for each persisted SQL-bearing object type found in the inventory.

```bash
cargo nextest run -p kalamdb-dialect --test postgresql_aliases
cargo nextest run -p kalamdb-core --test postgresql_upgrade
```

## 6. Architecture, security and resource guards (US6)

```bash
python3 scripts/check-sql-parser-boundaries.py
cargo nextest run -p kalamdb-dialect --test postgresql_architecture
```

The new guard rejects alternative production dialect constructors, raw user-SQL DataFusion text entry, independent command string dispatch and AST-to-text-to-AST loops. Test-only reference fixtures are explicitly scoped exceptions. Exercise depth/size limits, nested comments/quotes and original error positions. Assert no admin/system/tenant bypass through CTEs, views, UNIONs, aliases or prepared context changes.

## 7. Final compile, smoke and performance gates

Finish an edit batch, then capture one check and resolve all reported errors together:

```bash
cargo check -p kalamdb-server > /tmp/036-batch-check.log 2>&1
```

Start the backend in a separate terminal, with PostgreSQL wire configured for wire integration:

```bash
cd backend
cargo run --bin kalamdb-server
```

Prebuild CLI before e2e; do not use --no-fail-fast:

```bash
cargo build -p kalam-cli --bin kalam
cd cli
KALAMDB_SERVER_URL="http://localhost:3000" KALAMDB_ROOT_PASSWORD="mypass" cargo nextest run -p kalam-cli-e2e --test e2e smoke
```

Use the configured test root password. Finally rerun the identical performance harness:

```bash
cargo nextest run -p kalamdb-core --test postgresql_pipeline_perf
```

Attach results to `validation.md`; every matrix case needs evidence. Async tests use `#[ntest::timeout(...)]` at observed healthy runtime × 1.5. Missing server/toolchain/external docs is an outstanding validation/delivery item, not a passing gate.

## 8. Cleanup, injection and memory acceptance (SC-014–SC-017)

Populate cleanup.md with old symbols, replacements and callers; final production callers of removed paths must be zero. Re-run guards after deletion and verify retained aliases still use shared tokens. Capture resolved parser feature flags with `cargo tree -e features -i sqlparser` and prove recursive protection remains enabled.

New task suites:

```bash
cargo nextest run -p kalamdb-dialect --test postgresql_memory --test postgresql_custom_parse_once --test postgresql_hostile_input
cargo nextest run -p kalamdb-core --test postgresql_injection --test postgresql_memory_lifecycle
```

Injection cases include SQL-looking bound values and custom options, quoted identifiers with dots/quotes, single-command suffix smuggling, allowed batches with per-member authorization, SECURITY DEFINER host calls and cross-principal prepared reuse. Assert no unintended mutations and no secret leakage in application/dependency debug logs. Verify custom command options are never rendered into privileged SQL.

Run at least 10,000 mixed prepare/bind/drop/error/cancel lifecycles with short/large batches and several concurrent principals. Record concrete admission/cache limits, retained backing source/AST/plan bytes, clone counts and live ownership after cleanup. A tiny prepared statement must not retain a large unrelated batch. Oversized input must be rejected before expensive allocations; deep custom syntax and visitors must obey resource limits. Warm reuse has zero parses/source copies/full AST clones.

Extend the existing performance harness to each custom family and equal-work concurrency, measuring p50/p95 and allocation counts/bytes (within +10% per comparable case), peak admitted memory and runtime seconds. Replans and cache misses are measured separately. Run the new fuzz target for 120 seconds with its toolchain prerequisites, record outcome/runtime and retain minimized failures; unavailable fuzz tooling is an outstanding gate, not a pass. Use normal nextest for deterministic saved hostile/property cases. Security/fuzz tests do not establish an absolute proof that no vulnerability exists.
