# Data Model: PostgreSQL-Only SQL Language

**Feature**: 036-postgresql-sql-dialect  
**Date**: 2026-09-20

This is the parse-pipeline model. Catalog entities (namespace, table, user, procedure) already exist; this feature changes how SQL maps onto them.

## ParsedStatement

Single output of `parse_sql_statements` per semicolon-terminated command.

| Field | Type (logical) | Rules |
|-------|----------------|--------|
| `sql` | original slice | Unchanged text; used for errors and DataFusion fallback |
| `kind` | `SqlStatementKind` | Derived once from AST or extension payload; classifier does not re-parse |
| `ast` | `sqlparser::ast::Statement` | Present for standard SQL and for extensions represented as generic SQL |
| `extension` | `Option<KalamExtension>` | Present when dialect intercept built a Kalam-only command |
| `span` | start/end token | For error positions |

**Validation**: Exactly one of (standard `kind` from `ast`) or (`extension` with matching `kind`). Never both a homemade remainder string and an AST for the same command.

**State**: Immutable after parse.

## KalamExtension

Enum of Kalam-only commands parsed via `Dialect::parse_statement`. Each variant owns the existing struct (`CreateStorageStatement`, `SubscribeStatement`, `CreateTriggerOnTopic`, `CreateScheduleStatement`, `KillJobStatement`, …).

**Validation**: PostgreSQL quoting already applied by sqlparser tokenizer. No `trim().to_uppercase().starts_with` on the original string.

## TableKindSpec

How a `CREATE TABLE` declares user/shared/stream.

| Source | Maps to |
|--------|---------|
| `WITH (TYPE = 'USER'\|'SHARED'\|'STREAM')` | Canonical |
| `CREATE SHARED TABLE` / `CREATE STREAM TABLE` | Alias; inject TYPE |
| `CREATE USER TABLE` | **Invalid**; error, do not map |
| omitted TYPE | Existing default: **shared** |

**Invariant**: `CREATE USER` never enters this entity.

## SchemaIdent

PostgreSQL schema name ≡ Kalam namespace name.

| SQL | Entity action |
|-----|----------------|
| `CREATE SCHEMA [IF NOT EXISTS] ident` | Create namespace |
| `CREATE NAMESPACE …` | Same |
| `DROP SCHEMA` / `DROP NAMESPACE` | Drop namespace |
| `SET search_path TO ident` | Session default schema |
| `USE ident` / `USE NAMESPACE` / `SET NAMESPACE` | Same session default |

## UserIdentCommand

PostgreSQL identity DDL, not table DDL.

| SQL | Entity |
|-----|--------|
| `CREATE USER name WITH PASSWORD …` | User create (Kalam extras allowed in WITH) |
| `ALTER USER` / `DROP USER` | User alter/drop |
| `CREATE USER TABLE` | Syntax error (migration) |

## ProcedureCommand

034 function DDL parsed as PostgreSQL procedure shape.

| SQL | Notes |
|-----|--------|
| `CREATE [OR REPLACE] PROCEDURE name(args) … LANGUAGE ident AS $$body$$` | Intercept; not T-SQL |
| `CALL name(args)` | sqlparser `Statement::Call` |
| `GRANT EXECUTE ON PROCEDURE name TO role` | sqlparser Grant |

## ParseError

| Kind | When |
|------|------|
| `SqlSyntax` | PostgreSQL parser failed |
| `UnknownCommand` | Tokens are not SQL and not a documented extension |
| `RemovedSyntax` | `CREATE USER TABLE`, DuckDB lambda |
| `ExtensionSyntax` | Known extension with bad options |

**Invariant**: One error per statement. No “tried all extension parsers” aggregate.

## Relationships

```text
SQL text
  -> Tokenizer (PostgreSQL)
  -> KalamDbDialect.parse_statement? -> KalamExtension
  -> else Parser.parse_statement     -> ast::Statement
  -> map                             -> ParsedStatement { kind, ast, extension }
  -> classifier (pure)               -> same kind (auth, routing)
  -> executor                        -> handlers / DataFusion (dialect=postgresql)
```

## Deleted models

- `ExtensionStatement` as a string-prefix enum used for parsing
- Remainder-string `FooStatement::parse(&str)` as the source of truth
- Dual dialect pair `(classify=postgres, plan=duckdb)`
