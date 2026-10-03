# Contract: Public SQL Language

**Reference**: PostgreSQL 18, bounded by [capability-matrix.md](capability-matrix.md).
**Surfaces**: HTTP SQL single/batch, wire simple/extended, applicable WebSocket SQL, CLI/SDK SQL and bridge-generated SQL. Grammar is identical; transport operation availability and session lifetime can differ explicitly.

## Required canonical forms

Schema/table/view/index/type/policy/comment/grant, identities, procedures, queries, DML, transaction and session statements use PostgreSQL syntax as specified in FR-003 and the matrix. `CREATE TABLE` without TYPE remains shared. TYPE and other product table options are Kalam additions to PostgreSQL's WITH option syntax.

```sql
CREATE SCHEMA IF NOT EXISTS app;
CREATE TABLE app.items (id BIGINT PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE app.inbox (id BIGINT PRIMARY KEY) WITH (TYPE = 'USER');
INSERT INTO app.items VALUES (1, 'a');
SET search_path TO app;
SELECT name FROM items;
CREATE USER alice WITH PASSWORD 'secret';
```

Procedure examples in executable fixtures must create the procedure with its supported runtime before GRANT/CALL. Index/policy examples must use a table kind supporting those operations; unsupported combinations fail before mutation. No fixture relies on an undeclared `app.fn` or nonexistent mixed-case table.

## Extensions and retained aliases

All existing documented storage, compact, manifest, cluster, topic/consumer, subscription, schedule, backup/restore/export, job, and live-query operations remain. Exact options are inventoried from their owning modules and `docs/reference/sql.md` before cutover. Custom commands use PostgreSQL identifier/string/comment rules and the shared parser cursor.

| Compatibility form | Canonical form / behavior |
|---|---|
| CREATE/DROP NAMESPACE | CREATE/DROP SCHEMA for equivalent existing semantics |
| USE x / USE NAMESPACE x / SET NAMESPACE x | SET search_path TO x |
| CREATE SHARED/STREAM TABLE | CREATE TABLE WITH TYPE |
| DESCRIBE / product SHOW | Explicit Kalam inspection commands, not PostgreSQL standard grammar |
| AUTO_INCREMENT / AUTO INCREMENT | DEFAULT SNOWFLAKE_ID(), token-level alias only |
| CURRENT_USER() / CURRENT_ROLE() | PostgreSQL keyword form |
| Existing type/user extras | Documented typed Kalam options; no privilege escalation |
| JDBC `{call ...}` | CALL payload via token adapter when frozen client fixtures require it |

Retained aliases have no removal date in this feature. Reject CREATE/DROP USER TABLE with the canonical replacement. Reject DuckDB lambda execution, unregistered foreign syntax, trailing commas and bracket array constructors. `->` itself remains valid PostgreSQL JSON syntax. CALL LIMIT/OFFSET is rejected; no silent suffix stripping.

## Search paths and transactions

Preserve ordered entries, quoted identifiers and `"$user"`; resolve implicit `pg_catalog` as specified by the data model. Missing schemas are skipped for lookup; access checks remain enforced. RESET uses Kalam's configured default (`default` unless configured otherwise). SET LOCAL uses existing transaction lifecycle; outside a transaction it returns UnsupportedFeature without changing session state. Session state in a stateless HTTP request does not persist across requests. Unsupported temporary schemas, isolation modes, savepoints, SET options and unknown GUCs fail explicitly. Existing client metadata GUC no-ops must be individually enumerated and tested; never treat every unknown SET as success.

## Errors

Malformed SQL → SqlSyntax / wire 42601. Valid but unimplemented SQL → UnsupportedFeature / 0A000. Removed syntax → migration diagnostic (42601). Malformed known Kalam command → ExtensionSyntax (42601). Denied operations → Authorization / 42501. Existing resource-limit transport codes remain, with a stable ResourceLimit category. HTTP/WS expose equivalent categories and safe original positions. Neither malformed input nor unsupported options may be silently ignored or retried under another dialect.

## Compatibility evidence

The release corpus uses unchanged GUI/JDBC SQL and expected rows/metadata. Positive PostgreSQL lexical/JSON/array fixtures accompany negative dialect cases. Required operations execute across applicable entry points; parser-only acceptance cannot satisfy execution tests. Persisted SQL is scanned by upgrade preflight, changed explicitly via existing APIs, and verified after restart.
