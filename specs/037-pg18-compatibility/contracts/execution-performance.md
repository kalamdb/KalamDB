# Contract: Execution, Memory, and Performance Gates

This contract closes omissions found in the 2026-10-11 design review. It sets implementation acceptance criteria; it does not claim runtime verification or PostgreSQL performance parity. Owners remain #392–#400 as in the spec.

## One syntactic parse, dependency-aware planning

Keep an immutable parsed payload shared by classification, authorization and planning. Grammar comparison with sqlparser is a delegation test; PostgreSQL 18 itself is the semantic/syntax oracle. `PostgreSqlDialect` is not proof that all PostgreSQL 18 syntax is accepted or all other syntax rejected.

Count tokenization, syntax parsing, analysis, planning, and execution separately. Normal prepare/describe must not independently plan parameters and outputs. Invalidation or parameter-sensitive optimization can legitimately produce another plan from the same AST. Preserve the raw unresolved AST as well as resolved dependency identities; reanalysis after relevant schema, search_path, or type changes must not reuse stale bindings. Test PostgreSQL's prepared-statement behavior, including its documented same-name shadowing nuance. [PostgreSQL 18 PREPARE](https://www.postgresql.org/docs/18/sql-prepare.html).

Nested AST nodes are reused. SQL inside a string routine body is a distinct source unit, parsed according to the routine language's binding lifetime; runtime dynamic SQL is a new source unit per invocation. Do not eagerly resolve bodies that PostgreSQL resolves later. Parse-once prohibits redundant parsing of one source unit, not creation of new dynamic SQL. Source offsets refer to original input, including multibyte characters, comments, and dollar-quoted strings. Limit SQL bytes, tokens, statement count, AST depth, and recursive planning before resource exhaustion.

DDL parser migration tasks consume a shared cursor or existing typed node. Replacing every parser call with a new `KalamParser::parse(sql)` still reparses and fails G1. Bind values as typed parameters; never substitute them into SQL strings. Typed coercion/planner hooks are allowed; textual rewrite passes in hot paths are not.

## Cache correctness and memory

Extend `kalamdb-plan-cache`; do not create a parallel registry. Existing `PlanCacheKey` contains namespace, role and SQL, and existing Moka capacity is entry-based. These are insufficient by themselves for the expanded session/type contract.

- Parse-cache keys include syntax-affecting settings and parser revision; plan reuse also accounts for database, effective search path, parameter type OIDs, relevant session settings, catalog/type/function/collation revisions, and authorization/RLS dependencies.
- Share a plan across principals only when it contains no captured principal values and execution rebinds and revalidates identity/permissions. Otherwise partition it by principal and permission epoch. Test same-role users, SET ROLE, grants/revokes, policy changes, and prepared statements after schema changes.
- Dependency invalidation is an atomic epoch/snapshot operation. Never publish a partially updated catalog to plans or descriptions. Active executions pin immutable descriptors; reclamation waits for users of those descriptors.
- Bound entry count AND estimated retained bytes, with per-entry admission limits and per-session/global budgets for named statements and portals. Reject/bypass oversized entries. Account for AST strings, plan trees, schema graphs and buffers, including objects pinned after eviction.
- Use coalesced construction where safe to avoid a thundering herd. Do expensive planning outside map locks. Apply query admission/backpressure when pinned active work reaches the global budget; TTL is not a hard memory bound.
- Snapshot/transaction-bound data and mutable execution streams are not globally cached. No cached physical plan may carry a previous execution's parameters, security context, snapshot or mutable operator state.

## G4 transaction gate

G4 cannot close on one rollback and one concurrent write. Add implementation tasks and differential interleavings for each required row below. An exclusion stays a gap requiring the existing product decision process; this review does not approve exclusions.

| Required behavior | Minimum adversarial fixture |
| --- | --- |
| Atomic statement/batch | Error in row N of multi-row DML; earlier rows, indexes, RETURNING and events must follow the statement outcome |
| Failed transaction | Error after BEGIN; subsequent command rejected until rollback or appropriate savepoint recovery |
| Savepoints | Nested savepoint, rollback-to, release, name reuse, SET LOCAL and transactional DDL cleanup |
| Read Committed | Snapshot refresh between statements, own writes visible, concurrent update predicate recheck |
| Repeatable Read / Serializable | Stable snapshot, write conflicts, write skew/phantoms, serialization/deadlock errors and rollback; unsupported levels rejected explicitly |
| Constraints and sequences | Concurrent unique/FK checks; deferred constraints if supported; sequence gaps remain valid after abort |
| Commit and recovery | Cancel/disconnect before/after commit, crash/failover, no duplicate mutation after an ambiguous outcome |
| DDL and catalogs | Atomic rename/type/default/index changes, rollback, restart, active readers and prepared descriptions |
| Version domains | Cross-table/group atomicity inventory using domain-local frontiers; no global ordering invented from VersionId |
| Live delivery | Aborted writes emit no committed events; committed delivery preserves ordering/replay contract; overload produces explicit resync/close, never silent loss |

PostgreSQL isolates statements/transactions according to the selected level and sequence changes are not undone by rollback. Map these behaviors explicitly onto Kalam's commit domains rather than infer them from RocksDB atomic writes. [PostgreSQL 18 transaction isolation](https://www.postgresql.org/docs/18/transaction-iso.html).

No transparent DML retry after effects may have occurred. A pre-execution stale-plan retry is distinct from transaction replay. Serialization/deadlock retries belong at the whole-transaction boundary, only under an explicit safe retry contract. Volatile functions and external side effects must not execute twice accidentally.

## Protocol and HTTP sessions

Test named/unnamed statement and portal lifetimes, zero/one/per-column format codes, NULL parameters, unknown parameter types, invalid lengths and OIDs, repeated Bind, Describe before/after Bind, portal suspension/resume, Close, Flush, pipelining and disconnect. Bound message size and active portal buffers; stream rows under backpressure rather than collecting a complete result.

On extended-query error, drain to Sync and emit the correct ReadyForQuery transaction status; Sync does not roll back an explicit failed transaction automatically. Simple-query multi-statement messages must follow PostgreSQL's implicit transaction and error-stop rules, including explicit transaction boundaries. COPY success/failure, cancellation and portal exhaustion need state-transition tests. [PostgreSQL 18 message flow](https://www.postgresql.org/docs/18/protocol-flow.html).

HTTP SQL is stateless unless a documented session/transaction handle pins its database, identity, settings and transaction. Do not leak SET/search_path/transaction state through pooled connections between requests or users. Standard PG sessions keep their own state. Document unsupported HTTP multi-request transactions truthfully rather than simulating them.

### HTTP versus Wire session isolation and admission limits (T106, T107)

- **HTTP `/v1/api/sql` stateless isolation**: Each HTTP SQL request constructs a fresh `ExecutionContext` bound to the authenticated principal and a per-request `RequestTransactionBatchGuard` (`request_id`). Session settings (`SET search_path`) and explicit transactions (`BEGIN ... COMMIT`) live only for the duration of that HTTP request batch and are automatically rolled back on drop if uncommitted. No `search_path`, role, or uncommitted write state is shared across pooled HTTP connections or across users.
- **PostgreSQL wire session state (`WireConnectionState`)**: Each TCP/TLS wire connection owns an isolated `WireConnectionState` (`backend/crates/kalamdb-postgres-wire/src/connection.rs`) with its own `current_schema`, `transaction_id` (`BackendSessionManager` block transaction), bounded `PreparedStatementMap`, and bounded `PortalMap`.
- **Byte and slot admission limits**:
  - `max_statement_sql_bytes` (default `1 MiB` / `MAX_SQL_QUERY_LENGTH`): rejects oversized simple or extended query strings before parsing (`SQLSTATE 54000`).
  - `max_prepared_statements` (default `256`) and `max_prepared_statement_bytes` (default `4 MiB` per session): bounds retained AST/logical-plan metadata per connection; replacing an existing statement name reclaims its previous byte charge, and `Close` frees the slot and bytes immediately.
  - `max_portals` (default `64`): bounds concurrent open portals per connection (`SQLSTATE 54000` on exhaustion).
- **ReadyForQuery and aborted transaction state**: On extended-query failure, `pgwire` drains until `Sync` and `KalamQueryHandler::send_ready_for_query` queries `BackendSessionManager::block_state` to emit `'I'` (idle), `'T'` (open transaction), or `'E'` (aborted transaction block requiring `ROLLBACK` or `ROLLBACK TO SAVEPOINT`).

## Performance experiments before format commitment

T096 creates an executable harness under `benchv2/pg18/` and records fixed workload parameters and budgets before behavior changes. T005 unit tests establish correctness only; they cannot establish allocation or latency baselines. Every experiment compares:

1. Current KalamDB at the recorded base revision.
2. Candidate KalamDB with the same dataset/configuration/build profile.
3. Pinned PostgreSQL 18 on equivalent hardware, durability, data, indexes, isolation, connection transport and client settings.

No weaker durability, disabled auth, smaller results, or different semantics to improve a ratio. Report unavoidable architecture/configuration differences, including the dev Rust build versus distributed PostgreSQL binary; those comparisons are diagnostic, not production parity claims. Packaging-profile measurements are a separate explicitly labelled release exercise.

| Workload | Required measurement |
| --- | --- |
| Cold parse/prepare and hot repeated binds | Parse/analyze/plan counts, CPU, allocations, cache hit/miss, median/p95/p99 |
| Point read, indexed range, top-N | Rows/versions/bytes visited, plan shape, latency and throughput; preserve existing point-get path |
| Scan, join, aggregate, sort | Projection/predicate pushdown, estimates/statistics, partition pruning, spill bytes, peak RSS and query pool bytes |
| DML, COPY, RETURNING | Rows/s, commit latency, CPU/row, allocations/row, atomic failure behavior |
| Hot/cold/mixed versions, flush/compaction | Runtime seconds, encoded bytes, write amplification, memory, tombstone/visibility correctness |
| Catalog/DDL churn, many clients | Cache bytes, invalidation cost, throughput, p99, memory after old revisions are released |
| Narrow projection from large rows, slow readers | Retained owner bytes, result bytes, backpressure, time to release memory after Close/cancel |

Include 1, 8, 32 and 128 concurrent clients (or record a justified hardware cap), skewed parameters, mixed read/write, realistic null/text distributions, and data exceeding the configured memory budget to exercise spilling/admission. Keep optimization inside existing DataFusion providers/planner interfaces; do not build a second executor to imitate PostgreSQL. Statistics and index access often matter more than codec microseconds.

Initial acceptance targets, fixed before measurement and tightened only with evidence:

- At least 20% lower allocation bytes per row for the SELECT/flush materialization paths being replaced, and at least 10% lower CPU/row or median runtime for those targeted paths.
- No unexplained >5% regression in median or p95 of the representative workload set; no unexplained >10% regression in p99 or peak/retained memory. These are gates, not predicted results. Any necessary tradeoff must be explicitly reviewed before acceptance, with affected workload and reason.
- Report all encoded-size changes, especially null-heavy fixed-width rows. A >5% weighted storage increase blocks the format switch absent an explicit workload-based design decision; do not hide a severe individual regression behind the average.
- Memory remains within recorded global and per-query budgets under sustained churn and slow clients. Active queries may pin descriptors only within admission limits; released resources return to a stable plateau. Distinguish live allocations from allocator RSS caching.
- Report candidate/base and candidate/PG ratios per workload; a claim of being closer to PG performance requires an actual improvement in those ratios under equivalent work. No unmeasured overall parity claim.

Use warmup, at least five repeated samples for microbenchmarks, enough requests for tail quantiles, and alternating base/candidate runs. Save raw results, hardware/configuration, revision, dataset seed, confidence intervals/dispersion, failures and total runtime in seconds. Re-run noisy results; never silently widen budgets after observing a failure.

## Differential oracle and coverage

Fixtures compare values, logical types/OIDs/typmods, SQLSTATE, transaction status, and durable state. Compare ordered rows only where SQL guarantees order; otherwise compare multisets with duplicates. Never round decimals, lowercase quoted names, flatten arrays, strip timezone behavior, or ignore errors to normalize differences. Normalize custom OIDs by identity while checking uniqueness and references.

Migration validation includes typed content comparisons/checksums, NULLs, binary and temporal values, constraints, index definitions, sequence state and next generated values; row counts alone cannot detect corruption. Coverage for PostgreSQL 18 includes MERGE/RETURNING, old/new RETURNING, virtual generated columns, temporal constraints and SQL/JSON, each with its own matrix row rather than a general SQL pass. In particular, 18 introduces old/new RETURNING, virtual generated columns and temporal constraints. [PostgreSQL 18 release notes](https://www.postgresql.org/docs/18/release-18.html).

## Function and V8 extension

[functions-v8.md](functions-v8.md) adds T112/T122 to the baseline/final gate. Extend the same harness rather than create incomparable measurement reports: cold/warm calls, direct materialization, lazy read views, nested forwarding, binary transfer, full/sparse access, streams and concurrency. Track copied bytes per boundary and unique retained owner capacity alongside native/V8 heap and GC. Zero-copy claims require owner/copy evidence; a smaller allocation count alone is insufficient. SQL-equivalent routines use PG differential checks, while Kalam JS/TS performance is compared to its previous implementation.
