# Research: PostgreSQL-Only SQL Language

**Feature**: 036-postgresql-sql-dialect  
**Date**: 2026-09-20

## R-001: How to add Kalam commands without forking sqlparser

**Decision**: Implement `sqlparser::dialect::Dialect` for `KalamDbDialect`. Override `parse_statement(&self, parser: &mut Parser) -> Option<Result<Statement, ParserError>>`. Return `Some(Ok(...))` or `Some(Err(...))` when the upcoming tokens are a Kalam extension or a PostgreSQL statement sqlparser would parse with the wrong grammar. Return `None` to fall through to `PostgreSqlDialect` / default `Parser::parse_statement`.

**Rationale**: sqlparser 0.62 documents this as the custom-command hook. `Parser::parse_statement` calls it first. Adding variants to `ast::Statement` requires a sqlparser fork. DataFusion `DFParser` wraps `Parser` the same way.

**Alternatives considered**:

- Fork sqlparser to add `Statement::CreateStorage` — rejected (maintenance, workspace pin).
- Keep `starts_with` dispatch in `ExtensionStatement` — rejected (second lexer, comment/quote bugs).
- Parse with `GenericDialect` then hand-walk tokens — current classifier path; rejected as source of truth.

**Implementation notes**:

- Compose: `KalamDbDialect { inner: PostgreSqlDialect }` and forward identifier/keyword behavior to `inner` unless a hook needs extra keywords.
- sqlparser already tokenizes `STORAGE`, `CLUSTER`, `FLUSH`, `SHARED`, `STREAM`, `USER` as keywords. It does **not** know `NAMESPACE`, `SUBSCRIBE`, `SCHEDULE`, `TOPIC` as statement keywords — those must be intercepted as identifiers or added via dialect keyword helpers if needed.
- Do not implement custom `Statement` variants. After intercept, either:
  - return a generic `Statement` that converters already understand (`CreateTable`, `CreateSchema`, `Call`, …), or
  - parse into existing Kalam structs (`CreateStorageStatement`, …) stored beside the AST in `ParsedSql` (see data-model). Prefer wrapping by mapping to Kalam structs in `ParsedStatement` rather than stuffing Kalam data into `sqlparser::ast::Statement`.

## R-002: What sqlparser gets wrong under PostgreSqlDialect

**Decision**: Always intercept these prefixes in `KalamDbDialect::parse_statement`:

| Prefix | Upstream 0.62 behavior | Kalam behavior |
|--------|------------------------|----------------|
| `CREATE USER` / `ALTER USER` / `DROP USER` | Snowflake `CreateUser` (even with PostgreSqlDialect) | PostgreSQL role/user + Kalam `WITH` extras (`TENANT`, `NAMESPACE`, …) |
| `CREATE PROCEDURE` | T-SQL (`CREATE PROC`, `@params`, `BEGIN…END`) | PostgreSQL/Kalam `LANGUAGE … AS $$body$$` (034) |
| `CREATE TRIGGER` | Table trigger (`BEFORE INSERT ON table`) | Kalam `CREATE TRIGGER … ON TOPIC … EXECUTE PROCEDURE` when `ON TOPIC` is present; otherwise error or ignore until table triggers exist |
| `KILL` | `KILL <numeric connection id>` | `KILL JOB` / `KILL LIVE QUERY` only |
| `CREATE USER TABLE` | Would be `CREATE USER` then unexpected `TABLE` | **Reject** with migration message (spec FR-005) |
| `CREATE SHARED TABLE` / `CREATE STREAM TABLE` | Not PostgreSQL; likely parse error | Alias → `CREATE TABLE … WITH (TYPE=…)` |

**Rationale**: Falling through to upstream for these prefixes produces the wrong AST or a confusing parse error.

**Alternatives considered**: Pre-rewrite `CREATE USER TABLE` → `CREATE TABLE … WITH (TYPE='USER')`. Rejected because it keeps a colliding prefix in the public language. One-release reject with a clear error is the compatibility path.

## R-003: CREATE TABLE kind

**Decision**: Canonical syntax is already implemented:

```sql
CREATE TABLE name ( … ) WITH (TYPE = 'USER'|'SHARED'|'STREAM', …)
```

Remove regex stripping of `CREATE USER TABLE`. Default when `TYPE` is omitted remains **shared** (current `parser.rs` default). Keep `CREATE SHARED TABLE` / `CREATE STREAM TABLE` as dialect aliases that inject `TYPE`.

**Rationale**: Spec FR-005 / FR-006 / FR-019. `WITH (TYPE=...)` does not steal `CREATE USER`.

## R-004: Schema vs namespace

**Decision**: Parse `CREATE SCHEMA` with sqlparser (`Statement::CreateSchema`). Map to existing `CreateSchemaStatement` / namespace catalog APIs. `CREATE NAMESPACE` is a dialect intercept that produces the same mapped statement. Same for `DROP SCHEMA` / `DROP NAMESPACE` if both exist.

**Rationale**: PostgreSQL clients emit `CREATE SCHEMA`. Kalam catalog is namespaces. Alias avoids a silent rename of existing scripts.

## R-005: DataFusion dialect = PostgreSQL only

**Decision**: Set `datafusion.sql_parser.dialect` to `postgresql` in `kalamdb-core` session setup. Delete tests that require `x -> expr` lambdas. If `array_transform` remains, it must use DataFusion/PostgreSQL callable syntax, not DuckDB lambdas.

**Rationale**: Classification already uses PostgreSQL-ish rules. Planning with DuckDB is why `UNNEST` is rewritten (`pg_unnest.rs`) and why `->` is treated as DuckDB lambda vs PostgreSQL JSON.

**Alternatives considered**: Keep DuckDB dialect internally while advertising PostgreSQL publicly — rejected (spec FR-002, SC-008).

**Follow-up**: After switching, re-run GUI catalog `JOIN UNNEST(...)` queries. Keep `pg_unnest.rs` only if DataFusion+PostgreSQL still cannot plan them. Treat remaining rewrites as **internal**, not a public dialect.

## R-006: Classifier must not parse

**Decision**: `parse_sql_statements` returns `Vec<ParsedStatement>` (AST + optional Kalam extension payload + kind). Classifier becomes a pure function of that value. Delete the path that tokenizes with `GenericDialect` then calls `CreateSchemaStatement::parse(remainder)`.

**Rationale**: Two parsers disagree (comments, dollar quotes, `CREATE USER`). Spec FR-014.

**Fast path**: `SELECT` / `INSERT` / `UPDATE` / `DELETE` / `WITH` can still short-circuit **kind** assignment after the shared parse, but they must not skip producing the AST used by execution. Today some paths parse twice (classify then DataFusion). Target: classify uses the sqlparser AST; DataFusion may parse again internally (DataFusion owns its planner). Do **not** add a third Kalam string parser. Optional later: feed DataFusion an already-parsed statement if/when DF APIs allow; out of scope unless cheap.

## R-007: Statements to convert vs keep as converters

**Already AST-based (keep pattern, stop using remainder parsers)**:

- POLICY, COMMENT ON, GRANT/REVOKE (partial), CREATE TABLE after rewrite, ALTER TABLE, CREATE VIEW, DML visitors

**Convert from homemade parsers to sqlparser AST → existing structs**:

- CALL, CREATE SCHEMA, SET/RESET search_path, USE, GRANT EXECUTE, CREATE/ALTER TYPE (including Kalam extras `FROM TABLE`, `COMMENT`, `IF NOT EXISTS` via intercept if sqlparser lacks them), DROP TABLE, CREATE INDEX, DESCRIBE

**Stay dialect intercepts (not upstream Statement as-is)**:

- CREATE USER / ROLE (Kalam extras), CREATE PROCEDURE (034), CREATE TRIGGER ON TOPIC, STORAGE/CLUSTER/SUBSCRIBE/TOPIC/SCHEDULE/BACKUP/EXPORT/KILL JOB, CREATE NAMESPACE alias, CREATE SHARED/STREAM TABLE aliases

**Delete**:

- `parser/extensions.rs` `ExtensionStatement` prefix table
- `ddl/parsing.rs` leftover-string helpers used as parsers
- `user_commands.rs` Tokenizer loops
- `parser/system.rs` stub `SqlParser` if still unused
- `CREATE USER TABLE` rewrite in `create_table/parser.rs`
- DuckDB dialect assignment and lambda unit tests

## R-008: JDBC / pg_catalog regex in `parser/utils.rs`

**Decision**: After PostgreSQL-only parsing, re-evaluate each rewrite.

- JDBC escape `{call …}` — keep if clients still send it; it is a client protocol quirk, not a dialect.
- `pg_catalog` / `information_schema` regex rewrites — keep if shims still require them; they are catalog compatibility, not DuckDB dialect.
- DuckDB-specific `->` lambda protection — delete with DuckDB dialect.

## R-009: Contributor path

**Decision**: Rewrite `docs/development/how-to-add-sql-statement.md`:

1. If PostgreSQL already has the statement, add a converter from `sqlparser::ast::Statement` to Kalam structs + `SqlStatementKind`. Do not add a string parser.
2. If it is Kalam-only, add a `KalamDbDialect::parse_statement` intercept and a mapper.
3. Register execution in core/handlers as today.

## R-010: Breaking changes to document in SQL reference

- `CREATE USER TABLE` removed
- DuckDB lambdas removed
- Documented canonical forms: `CREATE SCHEMA`, `CREATE TABLE … WITH (TYPE=…)`, `CREATE USER` as identity
- Aliases listed: `CREATE NAMESPACE`, `CREATE SHARED TABLE`, `CREATE STREAM TABLE`, `USE NAMESPACE`

## Unresolved (implementation-time, not spec blockers)

- Exact DataFusion 55 PostgreSQL `UNNEST` support — measure in slice 5; keep or drop `pg_unnest.rs`.
- Whether sqlparser `CreateSchema` covers `AUTHORIZATION` / `IF NOT EXISTS` enough for Kalam — intercept if missing.
- `CREATE TYPE … FROM TABLE` extras — likely intercept after `CREATE TYPE` tokens.
