# Contract: PostgreSQL 18 Capability Matrix

This is the required release target, not an assertion that current code passes. Implementers record each fixture ID, transport, observed result and limitation in `validation.md`. `Required` means parse and execute the listed baseline; `Preserve` means freeze all existing working forms and add the named probes. Valid unimplemented variants return UnsupportedFeature (wire 0A000). A pinned-parser gap must be recorded distinctly; never claim full syntax support for it.

Fixtures use stable prefixes below; expand each into numbered positive/negative cases in `backend/crates/kalamdb-dialect/tests/fixtures/postgresql/`. Every applicable transport uses the same fixture ID. Transport-specific unavailable operations use UnsupportedFeature after shared parsing, not a different grammar.

| ID | Family | Parse/execution target | Explicit boundary | Stories |
|---|---|---|---|---|
| PG-DDL | Schema/table/view/index/type/comment/policy/grant | Required: forms in FR-003 with existing supported options, enum/composite types, procedure signatures, privileges and policy expressions | Unsupported constraints/options fail before mutation; never ignored | US1, US2 |
| PG-ROLE | CREATE/ALTER/DROP USER and ROLE | Required existing identity functionality using PostgreSQL password/login forms; token compatibility where upstream lacks representation | PostgreSQL superuser/replication/bypass-RLS powers cannot silently become Kalam privileges | US1 |
| PG-PROC | CREATE [OR REPLACE]/DROP PROCEDURE, CALL | Required existing Kalam procedure arguments, LANGUAGE, dollar body, SECURITY INVOKER/DEFINER; GRANT/REVOKE EXECUTE | Existing runtime languages only; PL/pgSQL execution unsupported; CALL LIMIT/OFFSET rejected | US1 |
| PG-DML | SELECT/INSERT/UPDATE/DELETE, RETURNING, ON CONFLICT | Preserve existing supported semantics; required insert/update/delete returning and conflict fixtures for currently supported forms | Unsupported conflict targets/actions explicitly rejected; no silent semantic loss | US3 |
| PG-QUERY | WITH/recursive CTEs, joins/LATERAL, subqueries/UNION, windows, FILTER, DISTINCT ON, ordering/limit/offset | Preserve existing executable forms, probe all named forms and execute those supported by the pinned planner/providers | Planner gaps recorded per exact form as UnsupportedFeature; no second dialect | US3 |
| PG-TYPE | `::`, CAST, integer/numeric/text/bool/date/time/timestamp, UUID, arrays | Preserve supported type mappings and typed defaults/parameters; probe precision/null behavior | Unsupported precision/type semantics explicitly rejected and listed | US1, US3 |
| PG-ARRAY | ARRAY literals, subscripts, ANY/ALL, UNNEST including catalog joins | Required existing working forms and pinned-DataFusion executable forms; unchanged GUI UNNEST fixtures | No DuckDB lambda fallback; remaining planner gaps explicit | US3 |
| PG-JSON | JSON/JSONB casts and `->`, `->>`, `#>`, `#>>`, containment | Required grammar; preserve existing executable operators, probe remaining planner capability | Unsupported JSON execution is 0A000, never a removed-lambda error merely for `->` | US3 |
| PG-LEX | quoted/unquoted identifiers, escapes, dollar tags, nested comments, batches | Required PostgreSQL lexical behavior and case folding through parse, catalog, resolution and auth | Existing product name/length limits explicitly reported; no raw quoting in domain IDs | US1, US3 |
| PG-SESSION | ordered search_path, SET SESSION/LOCAL, RESET/SHOW | Required ordered lookup, current-user schema expansion, implicit pg_catalog and transaction-local state | Kalam default is `default`; temporary schemas unsupported; inert client metadata GUCs must be enumerated, unknown settings rejected | US1, US3 |
| PG-TX | BEGIN/START TRANSACTION, COMMIT, ROLLBACK | Required existing transaction engine semantics through shared AST; test rollback and batch errors | Savepoints, isolation/options not implemented by existing engine fail explicitly | US1 |
| PG-PREP | `$1` parameters, wire Parse/Bind/Describe/Execute | Required typed binding, parameter/result metadata, cache/context invalidation | Extended Parse is one statement; no text substitution; unsupported parameter types fail explicitly | US3 |
| PG-EXPLAIN | EXPLAIN and supported options | Preserve existing planning/execution explanation | Unsupported options explicitly rejected | US1, US3 |
| PG-GUI | pg_catalog/information_schema/JDBC/GUI probes | Required unchanged baseline query text, visible rows and column metadata | No replacement-query escape clause; actual client protocol quirks documented separately | US3 |
| PG-UNSUPPORTED | COPY, MERGE, sequences/identity, table triggers, partitioning, FDW DDL on Kalam SQL endpoint, extensions, PL/replication | Probe PostgreSQL grammar and retain any existing supported form; unsupported engine features return 0A000 | Known pinned-parser gaps get explicit fixtures/limitations; no promise to implement these engines in 036 | US1, US3 |
| KDB-TABLE | WITH TYPE and existing table options | Required all table kinds; shared default; shared/stream aliases | CREATE/DROP USER TABLE removed with migration diagnostics | US2 |
| KDB-OPS | storage/flush/compact/manifest, cluster, topics/consumer/ack/retention, schedules, backup/restore/export, jobs | Required existing documented commands/options through shared tokens | All existing registry branches inventoried; auth unchanged | US4 |
| KDB-LIVE | SUBSCRIBE/UNSUBSCRIBE and embedded queries | Required existing live-query surface using shared query/Expr AST | Existing live-query execution restrictions remain explicit | US4 |
| KDB-ALIASES | namespace forms, USE, DESCRIBE, product SHOW, AUTO_INCREMENT, context keyword calls, type/user extras | Required compatibility forms already implemented; token/AST mapping only | AUTO_INCREMENT → DEFAULT SNOWFLAKE_ID(); CURRENT_USER()/CURRENT_ROLE() → keyword forms; documented as Kalam, not PostgreSQL | US2, US5 |
| KDB-JDBC | `{call ...}` escape wrapper | Retain if present in frozen client corpus using same tokenizer/cursor and CALL payload | Driver escape syntax is an enumerated adapter, never regex preprocessing | US1, US3 |
| NEG-DIALECT | DuckDB lambdas, bracket array constructors, trailing commas, alternate dialect settings | Required rejection; positive PostgreSQL operator/array controls accompany tests | Do not reject valid PostgreSQL expressions based solely on token spelling | US3 |
| UPGRADE | stored views/procedures/schedules and producer SQL | Required legacy detection, explicit migration, restart validation | No silent rewriting, data loss, or permanent legacy parser | US5 |

## Cross-cutting safety and efficiency fixtures

Add `SEC-INJECT`, `MEM-LIMIT`, `PARSE-ONCE` and `CLEANUP` fixtures to the same manifest. They apply to every standard/compatibility/custom family and transport: typed data/identifier boundaries, secret-safe diagnostics, per-member authorization, bounded admission/retention, cancel/drop/eviction and per-request parse counters. Follow [efficiency-security-cleanup.md](efficiency-security-cleanup.md). Successful parsing is one batch tokenization and one parse per statement; rejected input is at most one attempt, with zero for pre-admission rejection.

## Matrix completion rules

1. Setup freezes existing behavior for Preserve rows and client fixtures before changing the dialect; exact outcomes become committed regression fixtures.
2. Probe rows must enumerate each named construct as executable or explicitly unsupported with evidence from the pinned planner; a broad untested label is insufficient.
3. PostgreSQL syntax accepted by upstream but outside supported execution becomes UnsupportedFeature. Syntax upstream cannot represent is a recorded parser limitation; add compatibility nodes for Required rows.
4. No newly discovered working PostgreSQL form may be removed merely because absent from this table. Add it to the inventory and preserve it.
5. Explicit Kalam extensions and aliases are the only intentional grammar additions. They share lexical rules and cannot hijack valid PostgreSQL commands.
6. Corpus tests cover HTTP single/batch, wire simple/extended, applicable WebSocket operations, CLI and SQL-producing SDK/bridge paths. Native typed FDW calls get execution-parity tests, not artificial SQL parsing.
