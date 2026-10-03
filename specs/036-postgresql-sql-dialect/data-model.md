# Data Model: PostgreSQL-Only SQL Language

**Revised**: 2026-10-04. These are design contracts, not claims of implemented models.

## ParsedStatement

File: `backend/crates/kalamdb-dialect/src/models/parsed_statement.rs` (new).

| Field | Type | Invariant |
|---|---|---|
| source | `Arc<str>` | Original active-batch text; cached statements retain only their bounded original fragment or decline admission |
| source_origin | `SqlSourceOrigin` | Original byte/line/column origin for a compacted source fragment; separate model file |
| span | `SqlSourceSpan` | Original statement bounds for diagnostic mapping |
| payload | `StatementPayload` | Exactly one variant |

Immutable and role-independent. No parallel optional `ast`/`extension` fields. Classification is a pure projection; prepared metadata may cache the derived kind but cannot contradict its payload.

## StatementPayload

File: `models/statement_payload.rs` (new, relative to dialect `src/`).

| Variant | Payload | Use |
|---|---|---|
| Sql | `Box<sqlparser::ast::Statement>` | Faithfully represented standard SQL and canonicalized aliases |
| PostgresCompat | `PostgresCompatStatement` | Required PostgreSQL syntax not faithfully represented by pinned upstream AST |
| Kalam | `KalamStatement` | Product extensions with existing typed domain payloads |

`PostgresCompatStatement` and `KalamStatement` each have a separate model file. Reuse existing per-command structs when they preserve syntax; do not store unparsed remainder strings. PostgreSQL compatibility variants identify a documented upstream gap; they are not a second public dialect. Embedded AST expressions are retained, not rendered and parsed again.

## SqlStatementClass and bound domain commands

`models/sql_statement_class.rs` contains a small payload-free category enum for pure syntax classification. It does not duplicate every command payload. Existing `SqlStatementKind` embeds resolved domain structs, so AST-to-domain conversion builds that value only after execution-context name binding. Any syntax node whose existing struct requires a resolved `TableId` instead retains a typed unresolved object name until binding. Default schema, current role and tenant must not be frozen into shared parsed syntax.

## SqlSourceSpan and DialectError

Separate files `models/sql_source_span.rs` and `models/dialect_error.rs`. Span records original source offsets/locations; API adapters convert into transport-specific positions. Error variants are SqlSyntax, UnsupportedFeature, RemovedSyntax, ExtensionSyntax, Authorization, ResourceLimit. Diagnostics redact secrets; parser errors do not invent permission-sensitive catalog details.

## SearchPath

File: `backend/crates/kalamdb-commons/src/models/search_path.rs` (new). Reuse typed namespace IDs; a separate `search_path_entry.rs` holds named-schema versus current-user-schema entries. Preserve order and quoted identifier values. This is authoritative session state, not a second cached default-schema string.

Existing backend session state owns session values and transaction-local overrides. RESET restores the connection/request default (Kalam `default`, an explicit difference from stock PostgreSQL defaults). `SET LOCAL` requires an active transaction and is restored on transaction exit; session changes made in an aborted transaction roll back. Resolve `"$user"` only when that schema exists and is accessible. `pg_catalog` is searched implicitly before explicit schemas unless placed explicitly. Temporary-schema features are unsupported in this feature. Unqualified creation uses the first existing usable explicit schema; lookup uses ordered candidates with existing access checks. Stateless HTTP state expires with the request.

## PreparedExecutionStatement integration

Existing file: `backend/crates/kalamdb-core/src/sql/executor/prepared_execution_statement.rs`.

Replace duplicate classified/DML syntax ownership with shared `Arc<ParsedStatement>` and derived execution metadata. Parameter values, current authorization and session resolution remain execution-local. Plan-cache entries carry context/version identity; invalidate and replan on changes without reparsing the shared syntax. Do not share a permission-bound plan across users merely because SQL text matches.

## Table kinds and aliases

`WITH (TYPE='USER'|'SHARED'|'STREAM')` maps to existing `TableType`; absent TYPE remains shared. Shared/stream aliases canonicalize directly to the same payload. Reject duplicate/conflicting/unknown options, invalid values and unsupported constraints before catalog mutation. `CREATE USER TABLE` and `DROP USER TABLE` produce RemovedSyntax. Identity commands never enter table-kind mapping.

## Stored definition lifecycle

Original SQL + existing object ID/version → validation by shared parser → immutable cached syntax → authorization/binding → execution. A definition version change invalidates syntax/plan caches. Upgrade preflight reports object ID and removed form, without executing or changing definitions. Its offline runner consumes exported JSON records (object kind, ID, version, SQL) from stdin before binary replacement; startup also checks before catalog mutations. Explicit migration writes canonical definitions through existing APIs; restart tests verify them. No implicit persisted data rewrite and no permanent legacy parser.

## Execution flow

```text
source → one tokenizer/parser → ParsedStatement
                               → pure classification
                               → context binding + authorization
                               → typed handler or DataFusion AST planning
```

## Retention and release

`models/sql_source_origin.rs` stores original location mapping without retaining unrelated batch text. Compact source only once at cache admission; payload ASTs and original span meaning remain unchanged. No parser/token-vector lifetime escapes the parse function. Cache reuse shares syntax; private planner clones occur only for necessary replans, never ordinary warm binds.

The actual plan-cache owner is `backend/crates/kalamdb-plan-cache/src/lib.rs`; core's plan_cache.rs re-exports it. Add admission/retained-byte accounting there and to existing prepared/definition cache owners, including source backing allocations. Bound concurrent parse work before token allocation. No unbounded SQL interning or extra global cache. Dispose request bindings, failed parses and canceled requests promptly; tests use retained-byte/live-ownership checks rather than RSS alone.
