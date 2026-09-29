# Contract: Parser Architecture

**Feature**: 036-postgresql-sql-dialect  
**Owner crate**: `kalamdb-dialect`  
**Consumers**: `kalamdb-core` (execute/plan), `kalamdb-api` (HTTP SQL), `kalamdb-postgres-wire`

## Public parse API

One function parses a SQL string into statements:

```text
parse_sql_statements(sql: &str) -> Result<Vec<ParsedStatement>, DialectError>
```

Rules:

- Lexer/parser dialect is `KalamDbDialect` (PostgreSQL + intercepts). No `GenericDialect` classify path.
- Semicolon splitting uses the PostgreSQL tokenizer (dollar quotes, quoted identifiers).
- Each `ParsedStatement` includes `SqlStatementKind` plus AST and/or `KalamExtension`.
- Classifier MUST NOT call `FooStatement::parse(&str)` for commands listed in spec FR-003.

## KalamDbDialect

```text
struct KalamDbDialect { inner: PostgreSqlDialect }

impl Dialect for KalamDbDialect {
  fn parse_statement(&self, parser: &mut Parser) -> Option<Result<Statement, ParserError>>;
  -- forward remaining Dialect methods to inner unless a keyword hook is required
}
```

`parse_statement` MUST:

1. Peek tokens.
2. If prefix is a colliding or Kalam-only command, parse it here (`Some(...)`).
3. Otherwise `None` (PostgreSQL default).

Intercept prefixes (minimum):

- `CREATE USER` / `ALTER USER` / `DROP USER` (identity; reject `CREATE USER TABLE`)
- `CREATE PROCEDURE` / `DROP PROCEDURE` (034 body)
- `CREATE TRIGGER` when `ON TOPIC` follows
- `CREATE NAMESPACE` / `DROP NAMESPACE`
- `CREATE SHARED TABLE` / `CREATE STREAM TABLE`
- `CREATE STORAGE` / `ALTER STORAGE` / `DROP STORAGE` / `STORAGE`
- `CLUSTER`
- `SUBSCRIBE` / `UNSUBSCRIBE`
- `CREATE TOPIC` / `DROP TOPIC` / topic variants as documented
- `CREATE SCHEDULE` / `DROP SCHEDULE`
- `KILL JOB` / `KILL LIVE QUERY`
- Backup/restore/export command prefixes as documented today

Returning `Some` MUST produce either:

- a sqlparser `Statement` that downstream converters already handle, or
- a parse error.

Kalam-only payloads that sqlparser cannot represent SHOULD be stored on `ParsedStatement.extension` by the crate’s parse wrapper (the wrapper may finish parsing with `Parser` helper methods, then map to `KalamExtension` without putting Kalam variants into sqlparser’s enum).

## Mapping layer

`ast::Statement` / `KalamExtension` → existing domain types:

| Upstream / intercept | Existing type |
|----------------------|---------------|
| `Statement::CreateSchema` | `CreateSchemaStatement` |
| `Statement::CreateTable` + WITH TYPE | `CreateTableStatement` |
| `Statement::CreateIndex` | `CreateIndexStatement` |
| `Statement::Drop` (table) | `DropTableStatement` |
| `Statement::Call` | `CallStatement` |
| `Statement::Set` / `Use` / search_path | existing search-path / use types |
| `Statement::Grant` / `Revoke` | grant types including EXECUTE |
| `Statement::Comment` | `CommentStatement` |
| `Statement::CreatePolicy` | policy types |
| Intercept CREATE USER | existing user command types |
| Intercept CREATE PROCEDURE | 034 procedure AST |
| Intercept STORAGE/… | existing extension structs |

## DataFusion

`kalamdb-core` session: `datafusion.sql_parser.dialect = postgresql`.

MUST NOT set `duckdb`. MUST NOT expose a session/GUC that switches public dialect.

DataFusion may parse SQL again for planning. Kalam MUST NOT parse with a third homemade lexer before handing SQL to DataFusion.

## Deleted APIs (MUST NOT remain as parsers)

- `ExtensionStatement::parse` string-prefix dispatcher
- `ddl/parsing.rs` helpers used to parse full statements from leftovers
- Per-command `Tokenizer::new` loops for `CREATE USER` / CALL / schema
- Classifier `GenericDialect` + remainder parse for FR-003 statements

## Contributor rule

New standard SQL: converter from `sqlparser::ast::Statement` only.  
New Kalam SQL: `KalamDbDialect::parse_statement` intercept + mapper.  
Forbidden: `sql.trim().to_uppercase().starts_with("CREATE FOO")` for either.

## Tests the architecture MUST have

- `CREATE USER alice WITH PASSWORD 'x'` → user kind, not table
- `CREATE USER TABLE t (id INT)` → removed-syntax error
- `CREATE SCHEMA s` and `CREATE NAMESPACE s` → same kind
- `CREATE TABLE t (id INT) WITH (TYPE = 'USER')` → user table
- `CALL foo(1)` with preceding `-- comment` → call kind
- `STORAGE FLUSH TABLE "MixedCase"` → extension with quoted name preserved
- `array_transform(arr, x -> x)` → removed/unsupported syntax (dialect or planner)
- Classify then execute uses PostgreSQL dialect only (no duckdb setting)
