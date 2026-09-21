# Contract: Public SQL Language

**Feature**: 036-postgresql-sql-dialect  
**Applies to**: HTTP `/v1/api/sql`, PostgreSQL wire simple/extended query, CLI SQL, extension-bridge SQL.

All surfaces MUST accept the same grammar for the same statement text.

## Canonical PostgreSQL forms (MUST work)

```sql
CREATE SCHEMA IF NOT EXISTS app;
CREATE TABLE app.items (
  id BIGINT PRIMARY KEY,
  name TEXT NOT NULL
);
CREATE TABLE app.inbox (
  id BIGINT PRIMARY KEY
) WITH (TYPE = 'USER');
CREATE TABLE app.events (
  id BIGINT PRIMARY KEY
) WITH (TYPE = 'SHARED');
CREATE INDEX ON app.items (name);
CREATE VIEW app.v AS SELECT id, name FROM app.items;
COMMENT ON TABLE app.items IS 'items';
CREATE POLICY p ON app.items FOR SELECT TO public USING (true);
GRANT EXECUTE ON PROCEDURE app.fn TO public;
SET search_path TO app;
BEGIN;
INSERT INTO items (id, name) VALUES (1, 'a');
COMMIT;
CALL app.fn();
CREATE USER alice WITH PASSWORD 'secret';
```

`CREATE TABLE` without `TYPE` remains a **shared** table (existing default).

## Canonical Kalam extensions (MUST keep working; not renamed)

These are not PostgreSQL. They MUST parse with PostgreSQL quoting/comments:

- `CREATE STORAGE` / `ALTER STORAGE` / `DROP STORAGE` / `STORAGE FLUSH …`
- Cluster join/leave/status commands as currently documented
- `SUBSCRIBE TO … [WHERE …]` / unsubscribe
- Topic create/drop and `CREATE TRIGGER … ON TOPIC … EXECUTE PROCEDURE`
- `CREATE SCHEDULE` / drop schedule
- Backup/restore/export as currently documented
- `KILL JOB` / `KILL LIVE QUERY`

Exact option lists stay as in `docs/reference/sql.md` except where this contract forbids a prefix.

## Compatibility aliases (MUST keep in this feature)

| Alias | Canonical |
|-------|-----------|
| `CREATE NAMESPACE` | `CREATE SCHEMA` |
| `DROP NAMESPACE` | `DROP SCHEMA` (if both exist today) |
| `USE x` / `USE NAMESPACE x` / `SET NAMESPACE x` | `SET search_path TO x` |
| `CREATE SHARED TABLE` | `CREATE TABLE … WITH (TYPE = 'SHARED')` |
| `CREATE STREAM TABLE` | `CREATE TABLE … WITH (TYPE = 'STREAM')` |

## Removed syntax (MUST error)

| Removed | Error MUST mention |
|---------|-------------------|
| `CREATE USER TABLE …` | Use `CREATE TABLE … WITH (TYPE = 'USER')` |
| `DROP USER TABLE …` | Use `DROP TABLE` |
| DuckDB lambda `x -> expr` in `array_transform` / `array_filter` / `array_any_match` | Unsupported; PostgreSQL syntax only |

## CREATE USER vs CREATE TABLE

```sql
-- identity (MUST succeed)
CREATE USER alice WITH PASSWORD 'secret';

-- table (MUST succeed)
CREATE TABLE inbox (id BIGINT PRIMARY KEY) WITH (TYPE = 'USER');

-- MUST fail
CREATE USER TABLE inbox (id BIGINT PRIMARY KEY);
```

## CREATE PROCEDURE (034)

MUST accept PostgreSQL-shaped function DDL already used by Kalam (dollar-quoted body, `LANGUAGE`, `SECURITY INVOKER|DEFINER`). MUST NOT require T-SQL `CREATE PROC` / `@arg` / `BEGIN…END`.

## Errors

- Standard SQL failure: SQL syntax error (token/position when available).
- Known extension with bad options: named extension error.
- Unknown text: unknown command / syntax, not a list of failed extension parsers.

## Non-goals (this contract)

- `STORAGE FLUSH` is not `VACUUM`.
- `SUBSCRIBE TO` is not `LISTEN`.
- Full PostgreSQL is not promised; only the listed canonical forms plus existing documented Kalam extensions.
