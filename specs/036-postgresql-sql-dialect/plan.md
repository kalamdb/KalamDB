# Implementation Plan: PostgreSQL-Only SQL Language

**Branch**: `036-postgresql-sql-dialect` | **Revised**: 2026-10-04 | **Spec**: [spec.md](spec.md)

## Summary

Provide one PostgreSQL 18-based language with documented Kalam extensions. All SQL enters `KalamParser` in `kalamdb-dialect`, using sqlparser's tokenizer and a PostgreSQL-preserving `KalamDbDialect`. The parser returns a typed sum of upstream SQL AST, PostgreSQL compatibility node, or Kalam command. Classification, authorization, parameter metadata, and execution consume that result. Standard query planning uses DataFusion's AST API; there is no text reparse or DuckDB fallback.

This is language unification, not an implementation of every PostgreSQL storage engine feature. The [capability matrix](contracts/capability-matrix.md) specifies required execution, explicit limitations, aliases, and rejection cases.

## Technical context

- Rust 1.94, edition 2021; workspace sqlparser 0.62.0 and DataFusion 55.1.0. Use the existing dependency cohort; no fork or new parser library.
- Ownership: `kalamdb-dialect` owns lexing, parsing, normalization of identifiers, syntax validation and pure payload-free classification; existing context-bound `SqlStatementKind` payloads are created during binding. Core owns authorization/orchestration and the AST-to-DataFusion adapter. `kalamdb-backend` owns session/transaction lifecycle; permission-aware resolution uses the existing session/provider boundaries.
- Models: one model per file; typed IDs, `TableId`, existing domain enums; `Arc<ParsedStatement>` for prepared metadata, typed bind values per execution, no mutable parser payload stored on a shared dialect.
- Stored SQL: retain text and definition version in existing stores; validate/parse through the same API. An offline example runner accepts exported definition records on stdin for pre-upgrade validation; startup uses the same check before catalog mutation. No storage-format rewrite or direct filesystem work in core.
- Build/test: default dev profile, `cargo nextest run`, batch compile feedback once per edit batch. CLI smoke requires a running server and a prebuilt CLI.
- Performance: SC-009 compares complete pipelines under equal work and warm prepared execution; record runtime seconds and allocations. No extra SQL rewrite passes.

## Constitution check

| Principle | Design obligation and validation |
|---|---|
| I Performance-first | One tokenization/parse, AST planning, shared immutable prepared data; parse-count tests and same-work latency/allocation baselines. |
| II Ownership | Language code stays in dialect; session changes stay in existing session owners; filestore/store APIs perform any persisted-definition access. |
| III Dependencies | Reuse pinned sqlparser/DataFusion APIs; dependencies only through root workspace declarations with minimal features if strictly necessary. |
| IV Validation/docs | Tests precede each story implementation; cross-transport corpus, SDK tests/docs, architecture and migration documentation ship together. |
| V Composable APIs | Shared parser/resolver contracts have no UI framework dependencies; UI completion consumes the documented language inventory. |
| Delivery constraints | Do not hand-edit generated outputs. External docs must be updated and validated or reported outstanding. |

Design satisfies these obligations; implementation evidence is pending. Re-check before implementation and release. No performance or ownership exception is pre-approved.

## Concrete parser decision

`Dialect::parse_statement` returns only upstream `Statement`; it cannot return arbitrary Kalam payloads. `KalamParser` therefore wraps a single sqlparser `Parser`, dispatches owned commands by token lookahead, and returns `StatementPayload`. Commands representable upstream can use a dialect hook; custom payloads and required PostgreSQL parser gaps use the wrapper. Both routes use the same parser cursor and lexical rules. All custom grammar remains in `kalamdb-dialect`.

Forward PostgreSQL dialect identity (`Dialect::dialect()`), lexical and capability methods, expression hooks and precedence behavior. A representative upstream PostgreSQL fixture corpus must behave identically under the wrapper except for explicitly registered extensions. `None` from a hook delegates to upstream parsing; it must not consume input. Unknown syntax is never retried under another parser.

## Integration inventory

Paths are repository-relative. New files are explicitly marked; follow the smallest existing owning module when additional call sites are discovered and record them in the inventory.

| Surface | Existing paths / proposed additions |
|---|---|
| Shared parse API | `backend/crates/kalamdb-dialect/src/dialect.rs`, `parser/mod.rs`, `parser/utils.rs`, new `parser/kalam_parser.rs`, `parser/classify_from_ast.rs`, `parser/compatibility.rs` |
| Typed models | New `backend/crates/kalamdb-dialect/src/models/{parsed_statement,statement_payload,postgres_compat_statement,kalam_statement,sql_source_span,sql_source_origin,sql_statement_class,dialect_error}.rs` and module exports |
| DDL/extensions | `backend/crates/kalamdb-dialect/src/ddl/`; replace raw-text parse bodies with AST converters or shared-cursor parsers; remove `parser/extensions.rs` and obsolete `ddl/parsing.rs` dispatch helpers |
| Batch/classifier | `backend/crates/kalamdb-dialect/src/batch_execution.rs`, `classifier/engine/core.rs`, `classifier/types.rs` |
| Core planning/cache | `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs`, `prepared_execution_statement.rs`, `parameter_binding.rs`, `sql/plan_cache.rs` (re-export), owner `backend/crates/kalamdb-plan-cache/src/lib.rs`; new `sql/ast_planner.rs` |
| Sessions/resolution | `backend/crates/kalamdb-backend/src/{session,manager}.rs`, core `sql/context/execution_context.rs`, `sql/executor/sql_executor/postgres_meta.rs`, `schema_registry/policy_table_resolver.rs`; new `backend/crates/kalamdb-commons/src/models/search_path.rs` |
| HTTP/WS | `backend/crates/kalamdb-api/src/http/sql/statements.rs`, `http/sql/execute.rs`, `ws/events/subscription.rs` |
| Wire | `backend/crates/kalamdb-postgres-wire/src/{statement,query,tx_control,connection}.rs`, `client_catalog/{postgres_set,postgres_show}.rs` |
| Nested/stored SQL | Core `functions/{host,executor,lifecycle,schedule_store}.rs`, `views/mod.rs`, dialect `ddl/{create_view,create_procedure,create_schedule,subscribe_commands}.rs` |
| CLI/producers | `cli/src/sql_batch.rs`, `session/batch.rs`, `workflow/schema/typescript/mod.rs`; SDK handwritten SQL producers and fixtures under `link/sdks/` |
| UI | `ui/src/components/sql-studio-v2/input-form/sqlCompletionCatalog.ts` and its test |
| PostgreSQL extension | `pg/src/{fdw_ddl,remote_executor}.rs`, `pg/tests/e2e_ddl/`; typed FDW operations stay typed, SQL emitted by adapters is covered by fixtures |
| Documentation | `docs/reference/sql.md`, `docs/development/how-to-add-sql-statement.md`, new `docs/architecture/postgresql-dialect.md`, new `docs/migrations/postgresql-dialect.md` |

Do not mistake a PostgreSQL server's native parser in `pg/` for a second Kalam parser. Its SQL-emitting bridge must produce canonical SQL; non-SQL typed requests are exempt from parse counts.

## Delivery phases

1. Setup: freeze source inventory, unchanged GUI fixtures, capability fixture IDs, and pre-change performance baselines.
2. Foundation: models, PostgreSQL delegation, parser wrapper, pure classifier, stable errors, token-based batch boundaries and AST planner API. Keep the new pipeline internal until the atomic cutover; do not create a user-selectable dialect flag.
3. US1: standard DDL, required upstream PostgreSQL gaps, session/search-path/transaction behavior and authorization using typed results.
4. US2: canonical table options/defaults, complete constraints/options validation, shared/stream aliases, removed user-table forms.
5. US3: move core, HTTP and wire planning/metadata to ASTs, parameter binding/cache isolation, arrays/operators, catalog adaptations, strict syntax and dialect lock. Run all earlier story tests at cutover.
6. US4: finish custom-command families, subscriptions and nested stored SQL on the same token/parser API; remove old dispatch.
7. US5: aliases, producers, upgrade detection/migration fixtures, SDK and canonical docs.
8. US6: contributor/architecture contract and automated guards.
9. Release validation: complete corpus, authorization, CLI smoke, wire/bridge/SDK/UI tests and performance; no legacy parser left serving user SQL.

US1/US2 can be tested through the internal typed executor before US3 cutover. They are independently testable slices, not separately releasable dialect modes. The first PostgreSQL migration MVP comprises Foundation + US1 + US2 + US3; public replacement is not shippable until US4/US5 preserve extensions and migration safety.

## Bounded compatibility adaptations

Keep required catalog providers/UDFs. Convert existing lexical regex rewrites to AST/planner transformations where the unchanged fixture corpus proves they are needed; remove redundant ones. No new text preprocessing or retry-parse pass, including JDBC calls, context functions or UNNEST. If a parser gap needs token syntax support, add one minimal documented shared-cursor implementation with a regression fixture. Unmeasured additional hot-path complexity blocks the gate.

## Validation and completion

Use [quickstart.md](quickstart.md) for executable commands and expected results, [tasks.md](tasks.md) for dependency order, and the [parser contract](contracts/parser-architecture.md) for invariants. Update the matrix with observed fixture results during implementation. A planning checklist is not proof of runtime compatibility.

## Cleanup, memory, upstream reuse and injection controls

The [efficiency/security/cleanup contract](contracts/efficiency-security-cleanup.md) is a release gate, not optional polish. Expand the seeded [cleanup ledger](cleanup.md) with verified old-to-new symbol mappings and caller evidence; delete old production paths in each converted slice. Reuse sqlparser's token/helper/visitor APIs for standard and custom syntax, with no reimplemented lexer, expression grammar or parallel AST hierarchy.

Use safe owned Rust with shared immutable syntax, bounded admission before token allocation and byte-bounded existing caches. Avoid retaining large batch sources for small prepared statements. Inspect dependency raw-SQL logging as well as application diagnostics. Custom commands, error paths and nested SQL join the same parse-count, allocation, lifetime and injection corpus as standard queries. Bind data values; authorize typed identifiers and every batch member. Parsing is not a security boundary.

Runtime security and memory-safety claims require the executable evidence in SC-014–SC-017. The planning update does not assert that the current code is injection-free or leak-free.
