# Implementation Plan: PostgreSQL-Only SQL Language

**Branch**: `036-postgresql-sql-dialect` | **Date**: 2026-09-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/Users/jamal/git/KalamDB/specs/036-postgresql-sql-dialect/spec.md`

## Summary

Make PostgreSQL the only public SQL language on every KalamDB SQL surface. Replace homemade statement parsers with sqlparser 0.62 `PostgreSqlDialect` plus a real `KalamDbDialect` that intercepts Kalam-only commands and PostgreSQL statements that upstream models incorrectly (`CREATE USER`, `CREATE PROCEDURE`, topic `CREATE TRIGGER`). Classify from one parse tree. Point DataFusion `sql_parser.dialect` at PostgreSQL, not DuckDB. Reject `CREATE USER TABLE` and DuckDB `x -> expr` lambdas. Keep `CREATE SCHEMA` canonical with `CREATE NAMESPACE` as an alias; keep table kind as `CREATE TABLE ... WITH (TYPE = ...)`.

## Technical Context

**Language/Version**: Rust 1.94 (workspace edition 2021); sqlparser 0.62.0 (workspace pin)

**Primary Dependencies**: `sqlparser` (`Parser`, `Dialect`, `PostgreSqlDialect`, `Tokenizer`, `ast::Statement`); DataFusion 55.x session SQL parser dialect; `kalamdb-dialect`, `kalamdb-core`, `kalamdb-api`, `kalamdb-postgres-wire`

**Storage**: N/A (parse/classify/plan layer). Table/schema catalogs unchanged except how DDL is recognized.

**Testing**: `cargo nextest run -p kalamdb-dialect`; targeted `kalamdb-core` SQL/parser tests; CLI e2e smoke after server start (`./cli/run-tests.sh` or documented smoke filter); wire/HTTP SQL regression from 033 quickstart catalog probes

**Target Platform**: Server process (HTTP SQL, PostgreSQL wire, extension bridge)

**Project Type**: Backend language layer (Rust workspace)

**Performance Goals**: Parse+classify of a simple `SELECT` within 10% of current skip-full-DDL hot path (SC-009)

**Constraints**: Constitution: crate ownership (`kalamdb-dialect` owns parsing/classification); no extra SQL rewrite passes in hot paths; `Arc` sharing; no `use` inside methods; `cargo nextest`; one `cargo check` per edit batch

**Scale/Scope**: ~25 homemade parsers in `kalamdb-dialect`; DataFusion session dialect in `kalamdb-core`; docs `docs/reference/sql.md`, `docs/development/how-to-add-sql-statement.md`

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- Performance-first: One parse reused by classifier and executor. No new rewrite pass for statements sqlparser already understands. Keep the SELECT fast path (do not run every extension parser).
- Crate ownership: All public SQL lex/parse/classify lives in `kalamdb-dialect`. Core only consumes classified kinds + AST. Do not put a second dialect in `kalamdb-core`.
- DataFusion for query processing: Execution still uses DataFusion; the change is **which sqlparser dialect DataFusion uses** (`postgresql`), not a new planner.
- Type-safe domain models: Map parser AST onto existing `SqlStatementKind` / DDL structs (`CreateTableStatement`, `CreateSchemaStatement`, …). Do not add stringly command enums.
- Storage boundaries: Unchanged.
- Testing: Dialect unit tests for each converted statement; core tests for DataFusion dialect and UNNEST; CLI smoke for `CREATE SCHEMA` / `CREATE USER` / `CREATE TABLE WITH (TYPE=...)`.

## Project Structure

### Documentation (this feature)

```text
specs/036-postgresql-sql-dialect/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── sql-language.md
│   └── parser-architecture.md
├── checklists/requirements.md
└── tasks.md          # created by /speckit-tasks, not this command
```

### Source Code (repository root)

```text
backend/crates/kalamdb-dialect/src/
├── dialect.rs                         # replace type alias with KalamDbDialect
├── parser/
│   ├── mod.rs                         # single parse_sql_statements entry
│   ├── extensions.rs                  # DELETE string dispatcher
│   ├── classify_from_ast.rs           # NEW: Statement -> SqlStatementKind
│   ├── kalam_commands.rs              # NEW: Dialect::parse_statement intercepts
│   ├── pg_unnest.rs                   # keep only if still required after PG dialect
│   └── utils.rs                       # drop JDBC/pg regexes that PG dialect makes unnecessary
├── classifier/engine/core.rs          # classify from one AST; no FooStatement::parse
└── ddl/                               # converters from sqlparser AST; delete parsing.rs helpers

backend/crates/kalamdb-core/src/
└── datafusion_session.rs              # sql_parser.dialect = postgresql

docs/development/how-to-add-sql-statement.md
docs/reference/sql.md
```

**Structure Decision**: Extend `kalamdb-dialect` in place. Introduce `KalamDbDialect` wrapping `PostgreSqlDialect`. Convert existing `*Statement` types from “parse from remainder string” to “from sqlparser AST”. Delete `ExtensionStatement`, `ddl/parsing.rs` string helpers, and `user_commands` Tokenizer loops.

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|-----------|------------|--------------------------------------|
| Dialect intercepts for `CREATE USER` / `CREATE PROCEDURE` / topic `CREATE TRIGGER` | sqlparser 0.62 maps those prefixes to Snowflake/T-SQL/table-trigger grammars even under `PostgreSqlDialect` | Using upstream `Statement::CreateUser` as-is would accept the wrong option grammar and still collide with `CREATE USER TABLE` |
| Optional internal UNNEST rewrite (`pg_unnest.rs`) | DataFusion + PostgreSQL dialect may still fail GUI `JOIN UNNEST(...)` | Exposing DuckDB dialect publicly to get UNNEST planning — rejected by spec FR-012 / FR-013 |
| Compatibility aliases (`CREATE NAMESPACE`, `CREATE SHARED TABLE`) | Existing tests/scripts | Deleting all aliases in the same change as PostgreSQL unification would mix two breaking migrations |

## Phase 0 Research (complete)

See [research.md](./research.md). Decisions:

1. Official custom-command API is `Dialect::parse_statement` (`Some` = handled, `None` = PostgreSQL fallback). Do not add `Statement` variants to sqlparser.
2. Pattern: wrap `Parser` like DataFusion `DFParser`, or map dialect-owned structs onto existing Kalam AST. Prefer mapping onto existing `kalamdb-dialect` statement types.
3. DataFusion session dialect becomes `postgresql`. DuckDB lambdas become unsupported.
4. Canonical table kind: `WITH (TYPE=...)`. Reject `CREATE USER TABLE`. Keep shared/stream prefix aliases.
5. Canonical schema: `CREATE SCHEMA`. Alias `CREATE NAMESPACE`.

## Phase 1 Design (complete)

- Data model: [data-model.md](./data-model.md)
- Contracts: [contracts/sql-language.md](./contracts/sql-language.md), [contracts/parser-architecture.md](./contracts/parser-architecture.md)
- Validation: [quickstart.md](./quickstart.md)

## Implementation Strategy (for /speckit-tasks and implementers)

Implement in this order so each slice is independently testable:

1. **Foundation**: Real `KalamDbDialect`; `parse_sql_statements` always uses it; classifier consumes AST; DataFusion dialect `postgresql`. Keep old homemade parsers temporarily behind the same kinds so tests stay green.
2. **De-collide PostgreSQL**: Intercept `CREATE USER`/`DROP USER`/`ALTER USER`; reject `CREATE USER TABLE`; map `CREATE SCHEMA`; table type from `WITH (TYPE=...)`.
3. **Convert standard SQL parsers** one statement family at a time (CALL, GRANT EXECUTE, COMMENT/POLICY already AST-based, CREATE TYPE, CREATE INDEX, DROP TABLE, SET search_path, USE, DESCRIBE).
4. **Convert Kalam extensions** to `Dialect::parse_statement` (STORAGE, CLUSTER, SUBSCRIBE, TOPIC, SCHEDULE, KILL JOB). Delete `ExtensionStatement`.
5. **Remove DuckDB public syntax**: delete lambda tests or rewrite to PostgreSQL; keep `pg_unnest` only if still needed; drop session `duckdb`.
6. **Docs + contributor guide**.
7. **CLI smoke + wire catalog probes**.

Do not land a half-converted command: each command is either fully on the shared parse path or still on the old parser, never both live for the same prefix.

## Constitution Re-check (post Phase 1)

Still pass: one parse, dialect crate owns language, no new storage, DataFusion remains planner, type-safe kinds preserved.

## Next

Run `/speckit-tasks` to generate `tasks.md`, then implement from that file.
