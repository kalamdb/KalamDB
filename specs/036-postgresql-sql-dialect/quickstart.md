# Quickstart: PostgreSQL-Only SQL Language

**Feature**: 036-postgresql-sql-dialect  
**Goal**: Prove one PostgreSQL public grammar: standard DDL works, `CREATE USER` is identity, table kinds use `WITH (TYPE=…)`, DuckDB lambdas fail, Kalam extensions still parse with PostgreSQL quoting.

## Prerequisites

1. Build from `/Users/jamal/git/KalamDB/backend`.
2. For HTTP/CLI checks: run `kalamdb-server`.
3. For wire checks: enable `postgres_wire`; `psql` available (same as 033).

## Phase 0: Baseline (before implementation)

```bash
cd /Users/jamal/git/KalamDB/backend
cargo nextest run -p kalamdb-dialect
cargo nextest run -p kalamdb-core -E 'test(datafusion_session) or test(unnest) or test(lambda)'
```

Record passes. Note any tests that assert DuckDB lambdas or `CREATE USER TABLE` — those MUST be rewritten in this feature.

---

## Scenario 1: PostgreSQL schema and table (US1)

HTTP SQL or `psql`:

```sql
CREATE SCHEMA IF NOT EXISTS app;
CREATE TABLE app.items (id BIGINT PRIMARY KEY, name TEXT NOT NULL);
INSERT INTO app.items VALUES (1, 'a');
SELECT name FROM app.items;
SET search_path TO app;
SELECT name FROM items;
```

**Expected**: Schema + table exist; both SELECTs return `a`.

---

## Scenario 2: CREATE USER is not CREATE TABLE (US1/US2)

```sql
CREATE USER alice WITH PASSWORD 'secret';
CREATE TABLE inbox (id BIGINT PRIMARY KEY) WITH (TYPE = 'USER');
CREATE USER TABLE broken (id BIGINT PRIMARY KEY);
```

**Expected**: First two succeed (user + user-table). Third fails with a message pointing at `WITH (TYPE = 'USER')`. `system` user catalog contains `alice`. `inbox` is a user table, not a user named `broken`.

---

## Scenario 3: Aliases still work (US5)

```sql
CREATE NAMESPACE ns_alias;
CREATE SHARED TABLE ns_alias.t (id BIGINT PRIMARY KEY);
```

**Expected**: Same object kinds as `CREATE SCHEMA` + `CREATE TABLE … WITH (TYPE = 'SHARED')`.

---

## Scenario 4: DuckDB lambda rejected (US3)

```sql
SELECT array_transform([1, 2, 3], x -> x * 10);
```

**Expected**: Error on HTTP SQL and on wire. Must not plan via DuckDB dialect.

---

## Scenario 5: Kalam extension quoting (US4)

```sql
-- flush comment
STORAGE FLUSH TABLE "MixedCase";
```

**Expected**: Comment ignored; identifier keeps mixed case per PostgreSQL quoting. Not a tokenize-on-whitespace miss.

---

## Scenario 6: Catalog probe (US3, SC-005)

Re-run the 033 GUI/catalog `UNNEST` / `pg_catalog` probe used before this feature (see `specs/033-unified-backend-pgwire/quickstart.md`).

**Expected**: Same usable result, or a documented replacement query if DataFusion PostgreSQL dialect requires it. Session dialect is not `duckdb`.

---

## Scenario 7: Dialect unit tests (SC-006)

```bash
cd /Users/jamal/git/KalamDB/backend
cargo nextest run -p kalamdb-dialect
```

**Expected**: Coverage for CALL, CREATE SCHEMA, GRANT EXECUTE, COMMENT ON, CREATE POLICY, SET search_path, CREATE TABLE WITH TYPE, CREATE USER vs CREATE USER TABLE. No leftover tests that parse those via `starts_with`.

---

## Scenario 8: CLI smoke (after server up)

```bash
cd /Users/jamal/git/KalamDB/cli
KALAMDB_SERVER_URL="http://localhost:3000" KALAMDB_ROOT_PASSWORD="mypass" \
  cargo nextest run -p kalam-cli-e2e --test e2e smoke
```

**Expected**: Smoke still passes. Any smoke SQL using `CREATE USER TABLE` is updated first.

---

## Performance note (SC-009)

Time parse+classify of `SELECT 1` in the dialect test harness before and after. Report both runtimes in seconds. After MUST be within +10% of before unless a documented exception is recorded in the PR.
