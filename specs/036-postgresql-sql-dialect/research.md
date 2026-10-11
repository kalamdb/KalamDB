# Research: PostgreSQL-Only SQL Language

**Revised**: 2026-10-04. Source/API inspection is complete for the architecture decisions below; runtime feasibility and baseline measurements are implementation gates, not claimed results.

## R-001: A dialect hook cannot return arbitrary custom ASTs

sqlparser 0.62 `Dialect::parse_statement` returns `Option<Result<Statement, ParserError>>`. Its Statement enum is fixed. Adopt a `KalamParser` wrapper with `StatementPayload::{Sql, PostgresCompat, Kalam}`; custom commands consume the same parser cursor. Upstream-representable aliases may use dialect hooks. Do not fork sqlparser, invent dummy Statements, or use mutable side channels.

Evidence: [Dialect::parse_statement](https://docs.rs/sqlparser/0.62.0/sqlparser/dialect/trait.Dialect.html#method.parse_statement). DataFusion 55.1 `src/parser.rs` uses its own statement enum and parser wrapper, demonstrating the pattern rather than extending sqlparser's enum.

## R-002: Preserve PostgreSQL behavior when wrapping

Delegation includes `Dialect::dialect()` identity plus lexical/capability methods and expression parsing/precedence. A struct containing PostgreSqlDialect does not inherit its implementations. Add an upstream-versus-wrapper corpus for casts, arrays, dollar strings, nested comments, JSON operators and identifiers. Disable independent permissive options unless registered as intentional extensions.

Evidence: [Dialect::dialect](https://docs.rs/sqlparser/0.62.0/sqlparser/dialect/trait.Dialect.html#method.dialect); installed sqlparser 0.62 `src/dialect/mod.rs` includes `parse_with_wrapped_dialect` forwarding identity.

## R-003: AST planning is available now

Installed DataFusion 55.1 `src/execution/session_state.rs` exposes `statement_to_plan(statement)`; `datafusion-sql/src/parser.rs` exposes `Statement::Statement(Box<sqlparser::ast::Statement>)`. Use these from the owning core adapter. Original plan language permitting a second parse or deferring AST reuse is superseded. The workspace pins sqlparser 0.62/DataFusion 55.1 as one cohort; verify type identity during the foundation compile check.

Existing reparse paths include core `sql/executor/sql_executor/mod.rs` and wire `statement.rs` metadata inference. Migrate them, including retries and prepared caches. PostgreSQL config is a defensive default, not the mechanism for ingesting user SQL.

## R-004: Required upstream gaps are explicit compatibility nodes

Installed sqlparser 0.62 parses CREATE USER with a non-PostgreSQL options model and CREATE PROCEDURE with conditional-statement bodies rather than the Kalam dollar-body contract. Required forms get typed shared-token compatibility parsing. GRANT PROCEDURE has upstream representation and should use AST conversion. Test exact CREATE/ALTER/DROP USER/ROLE, CREATE OR REPLACE PROCEDURE, signatures and SECURITY clauses rather than assuming all prefixes need overrides. Unsupported PostgreSQL role privileges are rejected rather than mapped to stronger Kalam roles.

## R-005: Scope is a capability matrix

PostgreSQL 18 is the syntax reference. Preserve/extend the executable subset in [contracts/capability-matrix.md](contracts/capability-matrix.md). It distinguishes grammar, execution, pinned-parser limitations and Kalam aliases. Full PostgreSQL storage/PL/replication is not promised. Required rows cannot be downgraded silently to make tests pass; revise the spec explicitly if feasibility disproves them.

References: [PostgreSQL lexical rules](https://www.postgresql.org/docs/18/sql-syntax-lexical.html), [search paths](https://www.postgresql.org/docs/18/ddl-schemas.html), [JSON operators](https://www.postgresql.org/docs/18/functions-json.html).

## R-006: Custom syntax inventory is larger than storage commands

Inventory all dispatcher branches, SQL producers and stored definitions. Include compact/manifest, topic consumer/ack/retention, schedule alteration, jobs, exports, namespace/session aliases and type/user extras. USE/DESCRIBE are compatibility commands, not PostgreSQL standard statements. AUTO_INCREMENT and CURRENT_USER() compatibility are explicit token-level aliases; bracket array literals, trailing commas and unregistered cross-dialect forms are rejected. Alias removal needs a separate announced migration.

## R-007: Catalog compatibility works on ASTs/plans

Freeze unchanged working GUI/JDBC queries with expected rows and result metadata. Keep necessary catalog providers/UDFs; migrate needed UNNEST/context-function/JDBC transformations into token handling or AST/planner adaptations. A query replacement does not prove compatibility. Preserve valid PostgreSQL JSON arrows while rejecting DuckDB lambda execution; use ARRAY literals in negative tests to isolate the actual issue.

## R-008: Sessions and transport parity are implementation work

Wire currently has independent string-based SET/transaction recognition and reparses for metadata. Core/HTTP have separate split/prepare paths. Consolidate syntax in the dialect crate, maintain state in existing owners and add ordered search-path/transaction-local semantics. Typed FDW transport operations are not SQL parse paths. Dynamic SQL passed through procedure host APIs is a new submission; stored SQL version caches are reused.

## R-009: Migration is explicit

Update active CLI/UI/SDK producers and examples. Detect removed forms in persisted SQL via a read-only upgrade preflight and give object-specific remediation. Do not automatically rewrite user code. Existing historical documents and negative fixtures are exempt from blanket text removal. External canonical skills and affected SDK docs remain deliverables; physical paths must be resolved, not assumed.

## R-010: Validation before cutover

Foundation work captures baseline behavior and equal-work performance. Parse-count assertions cover simple requests, batch members, metadata inference, bind/cache hits and retry paths. All required matrix cases need executable tests. Run focused nextest suites before broad smoke and bridge integration; record runtime seconds for performance tests. No unmeasured exception is pre-approved.

## R-011: Upstream reuse and its limits (checked 2026-10-04)

The [upstream custom parser guide](https://github.com/apache/datafusion-sqlparser-rs/blob/main/docs/custom_sql_parser.md) recommends a delegating wrapper. This supports keeping custom productions small while reusing upstream parsing.

The [pinned 0.62 README](https://raw.githubusercontent.com/apache/datafusion-sqlparser-rs/v0.62.0/README.md) distinguishes syntax from semantics, describes recursive protection, notes incomplete source spans, requires error-path tests, and describes baseline benchmarking. These are upstream facts; our specific cache/admission, injection-test and deletion gates are KalamDB design decisions. We use the project's required dev profile rather than copying upstream release-benchmark commands.

[Upstream visitor APIs](https://docs.rs/sqlparser/0.62.0/src/sqlparser/ast/visitor.rs.html) provide borrowed traversal, mutable traversal and early exit. Reuse those for applicable AST validation/adaptation instead of another tree-walking framework. Local pinned-source inspection also verified parse_identifier, parse_object_name, parse_data_type, parse_expr, parse_comma_separated, peek_token_ref and with_tokens_with_locations. Custom handlers delegate to these helpers on one cursor.

Pinned `src/parser/mod.rs` owns a Vec<TokenWithSpan>; try_with_sql tokenizes and emits a debug log containing SQL. Consequently our integration uses bounded upstream tokenization and token-vector handoff, audits dependency log settings and avoids retaining Parser/tokens after parsing. This is not a zero-copy parser. Byte-admission limits must precede token allocation; borrowed lookahead reduces avoidable token clones.

## R-012: Security and cache ownership

[OWASP's primary injection guidance](https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html) recommends parameterized values and validation for non-bindable identifiers rather than treating escaping as the main defense. Apply this to internal/privileged SQL, custom options and stored SQL, while preserving intentional SQL submissions under caller authorization.

Current core `sql/plan_cache.rs` is a re-export. The owner is `backend/crates/kalamdb-plan-cache/src/lib.rs`, using existing Moka caches with entry/idle limits. Add retained-byte accounting/admission there and preserve one cache owner. An Arc to an entire batch can retain unrelated source; cache admission must compact once or decline caching while keeping original span origins. Replans reuse syntax, but eviction or a new submission necessarily begins a new lifetime. The detailed obligations are in [efficiency-security-cleanup.md](contracts/efficiency-security-cleanup.md).
