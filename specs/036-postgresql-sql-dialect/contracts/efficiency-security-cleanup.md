# Contract: Efficient, Safe Parsing and Legacy Removal

**Revised**: 2026-10-04 | **Requirements**: FR-031–FR-036, SC-014–SC-017

## Reuse upstream instead of implementing SQL again

Use pinned sqlparser 0.62 APIs: `Parser::parse_identifier`, `parse_object_name`, `parse_data_type`, `parse_expr`, `parse_comma_separated`, keyword/expect-token helpers and reference-returning lookahead such as `peek_token_ref`. Preserve the active parser cursor and all original token spans. Use `Visit`/`Visitor` for read-only traversal and `VisitMut`/`VisitorMut` only for necessary AST changes; combine compatible walks and stop early on errors rather than running redundant full-tree passes. Measure any required consuming-planner AST clone.

The wrapper recognizes only an extension's distinguishing production, then delegates identifiers, lists, expressions and nested queries to upstream. Audit existing converters before adding a compatibility node. Track every pinned-upstream gap in research with the smallest failing fixture and official API/source reference. Updating a pinned dependency requires cohort compatibility validation, not copying its implementation into KalamDB. Do not import GenericDialect from upstream examples into production.

A custom command containing a WHERE expression, option value or query never renders that fragment and calls a fresh parser. A SQL-language dollar-quoted definition is separate SQL content with its own definition-version parse lifetime; an opaque non-SQL body is not inspected as SQL. New dynamic SQL submitted by a host API starts its own lifetime. AST reuse ends at cache eviction/process restart; memory bounds take precedence over retaining every past statement.

## Memory ownership and bounded work

- Parse one admitted batch with one token vector and one parser; drop both after producing typed statements. Share source and AST ownership during the active request. Borrow for classification/validation and use reference-returning lookahead when consumption is unnecessary. Do not clone or uppercase the whole SQL string to dispatch commands.
- Source spans must remain valid UTF-8 boundaries. Upstream AST spans are incomplete for some nodes: keep statement bounds from the tokenizer, preserve original locations, and omit unknown precision rather than invent positions or reparse for diagnostics.
- Caching a small statement must not pin an unrelated large batch. On cache admission, retain only that statement's bounded source fragment and original origin mapping, or decline cache admission. A single measured source-fragment copy at admission is allowed; warm binds/retries must not copy source. Count the full retained backing allocation when sharing cannot be avoided.
- Use one authoritative AST. Cache hits share Arc ownership; if the planner requires an owned AST, move uniquely owned trees and otherwise clone at most once per necessary replan. Do not retain duplicate whole trees in classification/DML/wire metadata. Binds and authorization state stay per-execution; never mutate a shared AST with user values.
- Extend the existing cache owner `backend/crates/kalamdb-plan-cache/src/lib.rs` and existing wire limits, not the core re-export or a new global cache. Bound entries and retained bytes, reject oversized entries, account for source/AST/plan ownership, and enforce aggregate parse admission before expensive allocation. Moka entry count alone does not establish a byte bound; eviction convergence must not substitute for admission control.
- Record concrete active limits and measurement methods in validation before implementation: request bytes, batch statements, nesting, concurrent admitted bytes/requests, per-entry retained bytes and cache budget. Reuse existing configuration; any new default/setting is documented and tested. Byte input limits apply before token-vector allocation; post-tokenization checks cannot be the only memory defense.
- Safe Rust only in added integration code. No new unsafe, raw-pointer AST caches, transmute, leaked buffers, shared mutable dialect payloads, or unbounded interning. Preserve sqlparser's recursive-protection feature and explicit recursion limits; custom nesting and visitors must obey bounded traversal too. Audit existing reachable unsafe/dependency boundaries rather than assuming safe wrappers prove dependencies safe.
- Test cancellation/error paths, drop/eviction, disconnect and concurrent principals. Verify live ownership/retained bytes with counters and Weak/drop assertions; RSS alone is not a reliable leak oracle because allocators retain freed pages.

SQL parsing remains an allocating operation with owned tokens/AST strings; this contract does not claim upstream is zero-copy or prove absence of all OOM scenarios. Resource bounds and measured regressions are the acceptance evidence.

## SQL injection and command boundaries

sqlparser is a syntax parser, not a sanitizer, semantic validator or authorization engine. SQL submitted to the SQL endpoint is intentionally executable under the caller's privileges. Injection occurs when data unexpectedly becomes executable syntax or crosses that boundary.

- Bind scalar/array/null values through existing typed parameters. Values containing quotes, semicolons, comments, dollar tags or SQL keywords must remain data. Validate indexes/types/counts; unsupported binding must fail rather than interpolate.
- Build internal object references from typed identifier components/ASTs, never `format!` or concatenation into privileged SQL. Parameters cannot replace identifiers; resolve and authorize names independently. Preserve quoted component boundaries, including dots inside quotes. Bridges that must emit SQL use structured AST rendering plus typed value binds; the receiver parses the new submission once.
- Storage URIs/options, tenant/user/schema names, filter values, procedure arguments, topic/schedule options and persisted content remain typed fields. Any option intentionally defined as SQL enters the explicit SQL API with its own privilege context and lifecycle.
- Single-statement interfaces require delimiter/EOF after the command; malformed suffixes cannot be ignored. Batch interfaces authorize each parsed member and preserve documented transaction semantics. Reject invalid duplicates/constraints/unsupported options before side effects, not merely after syntax acceptance.
- Revalidate authorization on prepared reuse, after role/tenant/search-path/schema changes and for SECURITY DEFINER procedure calls. Data parsed while privileged cannot later confer those privileges to a different execution context.
- Redact logs and errors, including upstream logs. Pinned `Parser::try_with_sql` logs raw SQL at debug level. Prefer explicit upstream tokenization once plus `Parser::with_tokens_with_locations`, and restrict dependency parser/tokenizer logging in production; test both application and dependency log capture with password-bearing DDL and bound secrets. Never render a full sensitive AST to Debug as an error fallback.

Required attacks are regression fixtures, not a broad blacklist: quoted/comment payloads as bound values; embedded quote/dot identifiers; custom string options containing `'; ...`; procedure/schedule data that resembles SQL; trailing command smuggling on single-command routes; cross-principal prepared reuse; CTE/view/UNION and search-path privilege bypass attempts. Assert literal round trips, expected error categories and absence of unintended mutations. Include legitimate multi-statement input as a control.

## Cleanup ledger and deletion gate

Expand the seeded `specs/036-postgresql-sql-dialect/cleanup.md` during setup with columns: old path/symbol, replacement upstream API or typed path, production callers, action, test evidence, completion. Remove obsolete pieces in the slice that eliminates their last caller; the final sweep verifies removal rather than deferring all cleanup.

Mandatory candidates: `parser/extensions.rs`; obsolete `ddl/parsing.rs` full-statement helpers; unused `parser/system.rs`; classifier word/string fallbacks; wire SET/transaction string parsers; legacy DDL normalizers; raw-text context/JDBC/UNNEST rewrites replaced by AST adapters; repeated tokenization and AST serialization/reparse; duplicate prepared/DML AST fields; dead exports/tests/feature flags and parser-only dependencies. Keep needed domain models, retained aliases implemented on the shared cursor, catalog providers and non-parser uses of dependencies. Do not delete unrelated code or historical/negative fixtures to achieve a text-search target.

Release requires zero obsolete production paths, no disabled legacy modules waiting for later removal, no temporary dual-parser flag and no custom lexer/parser copy. Add guard fixtures that detect reintroduction. Report removed/replaced symbols and dependency changes; lines removed alone is not a correctness metric.

## Performance and safety gates

Apply quickstart's fixed dev-profile baseline protocol to standard SQL and at least one case per custom family, short and long batches, large literals/IN lists, deeply nested accepted input and rejected oversized/deep input. Measure equal work and concurrency; record p50/p95, allocation counts/bytes, retained source/AST/plan bytes, peak admitted memory, AST clones, cache behavior and runtime seconds. p50/p95 and allocation counts/bytes may not regress more than 10% per comparable case. Do not replace an allocation measurement with process RSS.

Warm prepared reuse requires zero tokenizer/parser invocations, whole-source copies and full AST clones; replanning costs are measured separately. At least 10,000 mixed lifecycle iterations must remain within recorded admission/cache budgets and release parser-owned objects to the expected bounded cache state. Successful parses require exactly one tokenizer invocation per batch and one statement parse per member. Rejected inputs permit at most one attempt, and pre-admission failures perform zero parsing. Tokenizers and parsers are instrumented for every custom family, errors and concurrent requests; counter attribution is per request, not a racy global total.

Run deterministic hostile-input/property regressions in normal nextest CI. Add a fuzz target exercising the same wrapper and bounded AST visitors, seeded with standard/custom/Unicode/deep/malformed cases; run a bounded 120-second smoke campaign and retain minimized failures. Fuzzing is additional evidence, never proof of no vulnerabilities. No dependency/tool installation is part of this planning change.
