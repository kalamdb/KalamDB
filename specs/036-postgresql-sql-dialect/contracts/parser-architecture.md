# Contract: Shared PostgreSQL Parser Architecture

**Owner**: `kalamdb-dialect` | **Consumers**: core, HTTP, wire, subscription and stored-SQL execution

## Public API and immutable result

```rust
// Logical signatures; each model has its own source file.
parse_sql_statements(sql: Arc<str>) -> Result<Vec<ParsedStatement>, DialectError>
parse_sql_expression(sql: Arc<str>) -> Result<Expr, DialectError>
classify(statement: &ParsedStatement) -> SqlStatementClass
```

Expression parsing is only for independently submitted expression input, never a substring extracted from a statement already being parsed. Nested expressions use the active parser cursor. Classification produces a small payload-free category (query/DML/DDL/session/transaction/Kalam), not a second command AST. It does not resolve names, inspect roles, parse text, or mutate the AST. Existing `SqlStatementKind` variants contain context-bound domain payloads: build those only during AST-to-domain binding against the current execution context, then authorize. Never fabricate a default namespace while parsing or cache a bound TableId as context-free syntax.

`ParsedStatement` contains original source, a source span and exactly one `StatementPayload`: `Sql(Box<sqlparser::ast::Statement>)`, `PostgresCompat(PostgresCompatStatement)`, or `Kalam(KalamStatement)`. `kind` is derived rather than independently mutable. Prepared metadata retains `Arc<ParsedStatement>`; binds and authorization state are not stored in the shared syntax node.

## One tokenizer and one parser cursor

`KalamParser` owns one sqlparser `Parser` configured with `KalamDbDialect`. Tokenize once with upstream Tokenizer and hand the located token vector to `Parser::with_tokens_with_locations`; avoid raw SQL dependency logging from `try_with_sql`. Preserve parser options/unescaping and recursion protection. It consumes all statements and their delimiters directly; batch splitting must not independently lex and then reparse each text fragment. Preserve original spans despite comments and escaped strings. Empty/comment-only batches are empty results. Extended-wire Parse requires one statement; simple wire/HTTP batches permit multiple with existing transactional behavior.

The dialect delegates PostgreSQL `dialect()` identity, lexical/capability methods, expression hooks, operator precedence and identifier quoting. Dialect defaults are not automatically PostgreSQL defaults. Verify delegation against upstream PostgreSQL fixtures. Parser options must not independently enable unregistered syntax such as trailing commas.

Custom grammar uses token lookahead, exact keyword sequences and Parser helpers. Dispatch must not claim keyword spellings inside quoted identifiers, comments, literals, or dollar bodies. Lookahead distinguishing topic triggers from table triggers must not consume tokens on fallback. Never attempt every custom parser or retry after a standard SQL parse failure.

## Upstream hook limitation and custom nodes

The real trait signature is:

```rust
fn parse_statement(&self, parser: &mut Parser)
    -> Option<Result<sqlparser::ast::Statement, ParserError>>;
```

Use it only where the result is faithfully representable as an upstream node. For arbitrary storage/topic/subscription commands or PostgreSQL procedure/user forms not represented upstream, the wrapper directly returns a typed owned variant using the same `Parser`. Do not return sentinel SQL nodes, stash per-parse payload on a shared dialect, fork sqlparser, or serialize custom nodes into SQL for reparsing.

Register all command families in one exhaustive typed dispatch surface in the dialect crate. Each command parser/model stays in its own owning module. Required families include all existing storage/flush/compact/manifest, cluster, subscription, topic/consumer/ack/retention, trigger, schedule, backup/restore/export, jobs/live-query, namespace aliases, USE/DESCRIBE/SHOW aliases and user/role extras. Existing parser inventory, not only this minimum list, determines completeness.

## AST planning and prepared statements

Core maps `StatementPayload::Sql(ast)` into DataFusion's `parser::Statement::Statement(ast)` and calls `SessionState::statement_to_plan`. Owned DDL/compatibility/Kalam nodes go to typed handlers. DataFusion 55.1 already provides this interface. The parser crate must not depend on the DataFusion planner.

`SessionConfig` uses `datafusion.sql_parser.dialect = postgresql` as a defensive internal default. User SQL must never reach `SessionContext::sql`, `sql_to_statement` or `create_logical_plan` with its original or regenerated text. Config/SET paths reject alternate-dialect selection, including resets/aliases that could restore a permissive default. Internal SQL, if unavoidable, enters the same shared parser; no allowlist for raw user SQL.

Use one permission-aware planning context for parameter and result inference. Bind typed values without textual substitution. Prepared cache identity/invalidation includes principal, tenant, role/permission changes, ordered search path, catalog/schema versions and statement identity. On invalidation reuse the syntax tree and rebind/replan; recheck authorization. Cloning a tree for a consuming planner API is allowed and measured; serializing/reparsing is forbidden. A definition change/new submission creates a new parse lifetime; ordinary binds/retries do not.

## Sessions, nested SQL and authorization

Transport adapters supply session context; they do not classify SET/SHOW/transactions with string prefixes. Existing backend session managers own state changes. Ordered search-path resolution must be shared by planning, DDL, catalog helpers and authorization. Missing privileges cannot be bypassed by quoted names, aliases, CTEs, views, UNIONs or search-path shadowing.

Views store parsed queries by definition version when cached. SQL embedded in procedures/schedules and SQL submitted by procedure host APIs use this parser; JavaScript or other non-SQL bodies remain opaque to the SQL parser. A dynamic SQL submission has its own parse lifetime. Subscriptions consume the same query/expression representation with the existing live-query restrictions.

## Errors and safety limits

One error includes category, original span and safe command name. Categories: SqlSyntax, UnsupportedFeature, RemovedSyntax, ExtensionSyntax, Authorization, ResourceLimit. Wire/HTTP/WS map equivalent categories; positions are original character positions, not rewritten byte offsets. Errors/logs must not expose passwords or protected object details. Preserve existing recursion limit (currently 512), request size limits and bounded parser resource behavior.

## Required deletion and guards

Delete old raw-prefix dispatch, per-command fresh tokenizers, Generic/DuckDB production parser selection and AST-to-text-to-AST paths after cutover. A CI guard inventories parser constructors and DataFusion text-planning calls with documented narrow exceptions for tests/reference fixtures only. Runtime parse counters prove one parse per statement and no reparse for cached binds/retries. Unit guards supplement, not replace, cross-transport execution tests.

## Mandatory efficiency/security/deletion contract

Follow [efficiency-security-cleanup.md](efficiency-security-cleanup.md) for upstream helper reuse, scoped source retention, bounded caches, safe Rust, typed binding, secret-safe logging, per-command parse counters and removal of obsolete paths. It applies equally to `Sql`, `PostgresCompat` and `Kalam` payloads. A separate submitted SQL body gets its own admitted parse lifetime; a predicate or option already in the active token stream never does.
