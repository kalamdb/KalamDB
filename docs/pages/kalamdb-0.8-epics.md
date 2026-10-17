# KalamDB 0.8 - Epics

> Canonical roadmap: maintain this file through repository pull requests. GitHub issues track implementation work and current status. The original ChatGPT Page is an archived planning snapshot, not an automatically synchronized copy.

KalamDB 0.8 will focus on PostgreSQL compatibility: aligning SQL syntax, matching PostgreSQL data types and behavior, and enabling PostgreSQL clients and tools to work with KalamDB. These epics define the work and release gates required to make that goal testable.

**Planning status:** Nine open GitHub trackers (#392–#400), checked on 10 October 2026, are organized below by priority and dependency. PostgreSQL 18 is the required baseline. The seven original epics plus the schema and catalog workstreams retain their detailed scope; this execution order replaces the earlier epic numbering. Owners, estimates and uncovered implementation tasks remain to be assigned.

## Compatibility contract

Create a versioned compatibility matrix covering SQL commands, types, functions, transactions, protocol messages, catalogs, and client workflows. Track each item as not assessed, missing, partial, or verified, with an owner and an automated test.

The target is full PostgreSQL compatibility against the agreed baseline. Any exclusions must be explicit product decisions; documenting a gap does not make it compatible. If 0.8 ships with gaps, describe its exact supported scope and retain full compatibility as an unfinished roadmap goal. Keep KalamDB-specific extensions documented and distinguishable from PostgreSQL behavior.

### Published 0.8 Compatibility Statement (matches `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`)

- **Verified PostgreSQL 18 scope**:
  - **One PostgreSQL dialect and AST-to-plan pipeline**: `KalamDbDialect` derives `PostgreSqlDialect` with `preserve_type_id = true`, parses user SQL once via `KalamParser`, and plans directly via `SessionState::statement_to_plan` across HTTP SQL and `pgwire` (`statement_plan_once`, `pg_syntax_corpus`, `kalam_extensions`).
  - **Schemas (`SchemaId`)**: `CREATE SCHEMA`, `ALTER SCHEMA`, `DROP SCHEMA`, `SET search_path`, quoted/Unicode identifiers, and `pg_namespace` catalog projection (`schema_slice`, `schema_Lookup`).
  - **Unified logical types (`LogicalTypeRef`) & Row Storage v2**: Built-in scalars, `NUMERIC(p, s)` / `DECIMAL(p, s)`, `JSON` vs canonical `JSONB`, `TIMESTAMP` vs `TIMESTAMPTZ`, arrays, named enums (`CREATE TYPE ... AS ENUM`, `ALTER TYPE ... ADD VALUE` / `RENAME VALUE`), composite types (`CREATE TYPE ... AS (...)`, 0-field composites, nested revisions), domains (`CREATE/ALTER/DROP DOMAIN` with defaults, `NOT NULL`, and SQL `CHECK (VALUE ...)`), and `ALTER TABLE ... ALTER COLUMN TYPE ... USING` backed by `StorageVersionLayout` and Row Body v2 (`pg_type_slice`, `pg18_type_semantics`, `pg18_domains_and_extensions`, `pg18_row_storage_v2`).
  - **Routines & V8 ABI v2**: Overload-safe `RoutineId`, PostgreSQL `CREATE/ALTER/DROP FUNCTION` and `PROCEDURE`, `IN`/`OUT`/`INOUT`/`VARIADIC` parameter binding, SQL/PLpgSQL/JavaScript execution, and zero-envelope `RoutineValue` native Arrow batch/row read views (`pg18_routine_syntax`, `pg18_routine_ddl`, `pg18_routine_binding`, `pg18_js_sql_calls`, `pg18_routine_context`, `pg18_routine_catalog`, `v8_regressions`, `runtime_lifecycle`).
  - **Execution, Transactions & Savepoints**: Explicit transactions, `SAVEPOINT` / `RELEASE SAVEPOINT` / `ROLLBACK TO SAVEPOINT`, `SQLSTATE 25P02` failed-transaction gating, `0A000` unsupported isolation level rejection, `INSERT/UPDATE/DELETE ... RETURNING`, and WebSocket subscription schema-change disconnect rules (`pg18_execution_slice`, `pg18_transaction_cases`, `pg18_subscription_schema`).
  - **Catalogs & Wire Protocol**: Shared `pg_catalog` (`pg_namespace`, `pg_class`, `pg_attribute`, `pg_type`, `pg_enum`, `pg_proc`, `pg_language`, `pg_attrdef`, `pg_description`, `pg_roles`, `pg_authid`, `pg_index`, `pg_constraint`, `pg_depend`, `pg_sequence`) and `information_schema` providers, extended query protocol (`Parse`/`Describe`/`Bind`/`Execute`/`Sync`/`Close`), per-session statement/portal/byte admission budgets, and ORM/GUI introspection (`pg_catalog_slice`, `wire_extended_query`, `pg18_migration_harness`).
  - **CLI Schema Diff & Lifecycle**: Unified project snapshot (`kalam-schema-diff`), topological dependency/risk planning, lexer-aware `-- UP`/`-- DOWN` markers, fenced migration receipts, and real-server project workflow (`pg18_project_snapshot`, `pg18_table_schema_diff`, `pg18_type_routine_diff`, `test_project_workflow_pg18`, `pg18_lifecycle_fault_injection`).
- **Explicit KalamDB extensions** (distinct from stock PostgreSQL 18): `CREATE TABLE ... WITH (TYPE='USER'|'SHARED'|'STREAM')`, `pg_catalog.file` (`FILE`) and `pg_catalog.embedding(N)` (`EMBEDDING(N)`), `LANGUAGE JAVASCRIPT` / `TYPESCRIPT` server routines with V8 host capabilities (`db.queryView`, `db.queryBatches`), and `SUBSCRIBE TO` live queries.
- **Explicit exclusions / documented gaps in 0.8**: Wire `COPY IN` / `COPY OUT` subprotocol and out-of-band `CancelRequest` packets (`missing`), `SERIALIZABLE` and `READ UNCOMMITTED` isolation levels (`rejected with 0A000`), and range/multirange/shell `CREATE TYPE` forms (`rejected with explicit diagnostic`).

## Priorities and execution order

Start with **#392 G1: one PostgreSQL dialect and DataFusion AST planning**. P0 establishes shared foundations and continuous verification; P1 delivers compatible behavior; P2 is final release readiness. All are required for the agreed 0.8 scope. Stage numbers are delivery order; issue numbers remain stable.

| Stage | Priority | Epic | Start condition |
|---|---|---|---|
| 1 — G1 foundation; grammar continues in stages 2–4 | P0 | [#392 DataFusion foundation and one PostgreSQL dialect](https://github.com/kalamdb/KalamDB/issues/392) | Start now. The G1 foundation does not depend on completion of #393 or #400. |
| 2 — G2 schema contract and migration | P0 | [#400 Replace namespaces with PostgreSQL schemas](https://github.com/kalamdb/KalamDB/issues/400) | #392 G1 for production migration; the identity/serialization audit can start during stage 1. |
| 3 — G3 unified type design and vertical slice | P0 | [#393 Unify PostgreSQL types on DataFusion and Arrow](https://github.com/kalamdb/KalamDB/issues/393) | #392 G1 and #400 G2 for the broad migration. Design and representation experiments can begin earlier without introducing a second permanent model. |
| 4 — G4 SQL and transaction semantics | P1 | [#394 Match SQL execution and transaction behavior](https://github.com/kalamdb/KalamDB/issues/394) | #392 G1, #400 G2 and #393 G3; implement against completed type families rather than waiting for every PG type. |
| 5A — G5 catalogs and introspection | P1 | [#399 Align pg_catalog and information_schema with PostgreSQL 18](https://github.com/kalamdb/KalamDB/issues/399) | #392 G1, #400 G2 and #393 G3. Use relevant #394 semantics for lifecycle/transaction tests. No dependency on completion of #395. |
| 5B — protocol and client integration | P1 | [#395 Support PostgreSQL connections and clients](https://github.com/kalamdb/KalamDB/issues/395) | #392 G1 and #393 G3; consume #400 schema/session interfaces and #399 catalog providers as each lands. No need to wait for all G5 catalogs to start protocol work. |
| 6 — tooling and migration | P1 | [#396 Support PostgreSQL tooling and migration](https://github.com/kalamdb/KalamDB/issues/396) | #394 ordinary workflow G4, #399 G5 and #395 required protocol/client slice. |
| Continuous from stage 1; final gate at stage 7 | P0 | [#397 Continuously verify PostgreSQL compatibility and regressions](https://github.com/kalamdb/KalamDB/issues/397) | Start now alongside #392. Establish the harness and baselines before behavior changes; final closure requires all functional epic evidence. |
| 8 — release readiness and sign-off | P2 | [#398 Make 0.8 ready for developers and production](https://github.com/kalamdb/KalamDB/issues/398) | #397 final gate, required #392–#396/#399/#400 acceptance evidence and resolved release blockers. |

Dependencies apply to the named gates below, not entire epic closures. #392's remaining type/routine grammar proceeds with #393/#394 after G1. #399 and #395 share stable contracts and integrate together; neither waits for the other's full closure. #397 starts on day one and supplies every gate's tests. Assign one accountable owner per epic and record estimates after each gap inventory; no owner or duration is assumed here.

## Start here: DataFusion foundation

**Architecture direction:** Use upstream sqlparser-rs with one PostgreSQL dialect, then pass one shared typed statement to DataFusion for planning, optimization and Arrow execution. Kalam commands use minimal typed extensions through the same parsing boundary. DataFusion/Arrow are the execution foundation; PostgreSQL 18 defines compatibility. A single shared logical type catalog supplies PostgreSQL identity, modifiers and custom-type semantics, with derived Arrow layouts and wire descriptors. Do not add another independent type enum, SQL parser or execution engine.

- [ ] Inventory every parser/tokenizer, SQL rewrite and text-to-plan entry point; separate production paths from explicit negative tests and genuinely internal SQL.

- [ ] Reuse the existing spec at specs/036-postgresql-sql-dialect after reconciling it with the current checkout. Use upstream sqlparser-rs with PostgreSqlDialect and minimal typed Kalam extensions; no second dialect or planner.

- [ ] Introduce the shared AST/payload boundary, then migrate a SELECT and parameterized DML through both transports, metadata and retries before expanding to all callers.

- [ ] Replace remaining GenericDialect paths and SQL-text planning with AST handoff; unify prepared metadata rather than planning the same statement separately for parameters and results.

- [ ] Migrate matching tests and fixtures in each change and remove superseded helpers. Attach G1 evidence before broad schema/type migration.

**G1 acceptance:** G1: one PostgreSQL parser configuration, one shared typed statement payload and AST-to-DataFusion planning across HTTP, pgwire, DML, prepared metadata, nested SQL and retries. No runtime legacy-dialect fallback or user-SQL text rewrite/reparse. Existing behavior remains covered; parse-count, malformed-input, allocation and retention checks pass.

**Code review snapshot — 10 October 2026:** Read-only inspection of local HEAD `ca3ac3a9c7569f298bc8646425c5207aadbba5f7` plus uncommitted changes; this is a planning assessment, not a passing test run. Cargo.lock resolves DataFusion 55.2.0, Arrow 59.2.0 and sqlparser 0.62.0. `kalamdb-dialect/src/dialect.rs` already aliases PostgreSqlDialect and `kalamdb-core/src/sql/datafusion_session.rs` sets PostgreSQL. Remaining runtime GenericDialect paths include classifier/engine/core.rs, executor DML metadata, create_view, alter_table and user commands. `kalamdb-postgres-wire/src/statement.rs` separately plans SQL text for parameter and result inference; executor paths also call session.sql, including retries. ColumnDefinition still combines KalamDataType, named_type_id and is_array; wire preparation and row encoding maintain separate mappings. Recheck these paths at implementation time because the checkout is being edited.

The installed DataFusion 55.2.0 source exposes `SessionState::statement_to_plan`; its DFExtensionType documentation currently lists pretty-printing as customizable. Verify actual APIs and semantic propagation in G3 before selecting extensions. Upstream [DataFusion type/metadata guidance](https://datafusion.apache.org/blog/2025/09/21/custom-types-using-metadata/) explains the Arrow foundation and metadata propagation; metadata alone is not proof of PostgreSQL behavior.

## Stage 1 · P0 · DataFusion foundation and PostgreSQL syntax — #392

**GitHub tracker:** [#392 Align SQL syntax with PostgreSQL](https://github.com/kalamdb/KalamDB/issues/392) — open.

**Start:** Start now. The G1 foundation does not depend on completion of #393 or #400.

**Gate:** G1: one PostgreSQL parser configuration, one shared typed statement payload and AST-to-DataFusion planning across HTTP, pgwire, DML, prepared metadata, nested SQL and retries. No runtime legacy-dialect fallback or user-SQL text rewrite/reparse. Existing behavior remains covered; parse-count, malformed-input, allocation and retention checks pass.

**Handoff:** G1 unlocks #400 G2 and #393 G3. The remaining grammar work in this issue proceeds alongside schemas, types and execution; full closure still requires those integrations.

**Related issues:** [#208](https://github.com/kalamdb/KalamDB/issues/208) CREATE TABLE accepts duplicate column names (case-insensitive) (closed).

**Outcome:** Developers can use PostgreSQL SQL without rewriting syntax for KalamDB.

- Inventory and implement PostgreSQL grammar, including identifiers and quoting, literals, casts, parameters, DDL, DML, joins, subqueries, CTEs, window queries, conflict handling, and returning clauses.

- Use one shared parsed representation throughout validation, authorization, planning, and execution. Parse each submitted statement once, including KalamDB commands; do not turn an AST back into SQL for reparsing.

- Reuse upstream parser helpers, keep KalamDB grammar additions minimal, and remove obsolete dialect fallbacks and SQL rewrite paths.

- Match statement validation and error locations; reject unsupported forms explicitly until implemented.

**Done when:** Every syntax item in the compatibility matrix has positive and negative tests. Accepted statements reach working execution, and tests verify the parse-once invariant.

## Stage 2 · P0 · Replace namespaces with schemas — #400

**Required migration:** [#400 Replace namespaces with PostgreSQL schemas across code and public interfaces](https://github.com/kalamdb/KalamDB/issues/400).

**Start:** #392 G1 for production migration; the identity/serialization audit can start during stage 1.

**Gate:** G2: one SchemaId-based schema contract, authoritative lookup/search_path interfaces, ownership rules and an explicit persisted-format/API migration decision. A CREATE SCHEMA → ordinary CREATE TABLE → lookup → restart slice passes before downstream consumers migrate.

**Handoff:** G2 unlocks schema-aware type/routine ownership in #393 and catalog providers in #399. Full schema semantics and acceptance continue with #394/#397; do not wait for the final application journey to expose the shared contract.

- [ ] Rename NamespaceId to SchemaId throughout production code, including related domain models, modules, fields, methods, stores, and services. Use one authoritative schema catalog; remove permanent aliases and parallel namespace/schema definitions.

- [ ] Adopt PostgreSQL 18 CREATE SCHEMA, ALTER SCHEMA, DROP SCHEMA, ownership/privileges, qualified names, public, and search_path behavior. Apply the same resolution to tables, types, enums, and routines.

- [ ] Use schema terminology across APIs, CLI, SDKs, Admin UI, diagnostics, templates, examples, and generated contracts.

- [ ] Preserve stable object identity and explicitly migrate affected serialized fields, storage keys, configuration, and protocol contracts. Temporary upgrade adapters require owners and removal milestones.

- [ ] Keep PostgreSQL's actual compatibility identifiers, including pg_namespace and its namespace-named catalog columns. They must not be renamed.

- [ ] Complete the deletion inventory and migrate all tests, fixtures, examples, and documentation; verify lookup, permissions, rename/move/drop, rollback, restart, and dependency cleanup.

**Developer experience acceptance:** An ordinary PostgreSQL developer must be able to connect with a standard driver and use schema DDL, ordinary CREATE TABLE, types/enums, typed functions, parameters, CRUD, transactions, and introspection without learning Kalam-specific syntax or using a Kalam SDK. Run the same fixture against PostgreSQL 18 and KalamDB, including psql and ORM workflows. Keep Kalam extensions optional and verify the supported behavior rather than claiming compatibility from appearance alone.

PostgreSQL references: [CREATE SCHEMA](https://www.postgresql.org/docs/18/sql-createschema.html), [schemas and search paths](https://www.postgresql.org/docs/18/ddl-schemas.html), and [pg_namespace](https://www.postgresql.org/docs/18/catalog-pg-namespace.html).

## Stage 3 · P0 · Unified PostgreSQL types — #393

**GitHub tracker:** [#393 Match PostgreSQL data types](https://github.com/kalamdb/KalamDB/issues/393) — open.

**Start:** #392 G1 and #400 G2 for the broad migration. Design and representation experiments can begin earlier without introducing a second permanent model.

**Gate:** G3: reviewed canonical type contract, owning modules and acyclic dependencies; derived Arrow/DataFusion layouts and pgwire descriptors; a proven numeric(12,2), jsonb, timestamptz, enum and composite-array slice through planning, storage and wire. Migration/deletion inventory and performance budgets are approved in the design review.

**Handoff:** G3 unlocks #394 execution and #399/#395 metadata/codec integration. The representative wire slice belongs to this gate and does not wait for full protocol epic closure.

**Related issues:** [#50](https://github.com/kalamdb/KalamDB/issues/50) SQL API: Add schema-aware array-rows response + queryRef/schemaRef caching (Kalam-link friendly) (open).

**Outcome:** PostgreSQL schemas and values retain their meaning when used in KalamDB.

- Inventory every PostgreSQL built-in type, alias, modifier, and type category.

- Implement numeric, character, binary, boolean, date and time, UUID, JSON, array, enum, composite, domain, range, network, geometric, bit string, XML, text search, and remaining catalog types.

- Match precision, scale, bounds, special values, time zones, null behavior, comparisons, implicit conversions, and explicit casts.

- Preserve type identity through storage, query results, SDKs, Admin UI, and PostgreSQL text and binary encodings. Cover type metadata and type identifiers used by clients.

**Done when:** A type-by-type test suite verifies boundary values, invalid inputs, casts, ordering, and round trips through storage and clients without silent truncation or precision loss.

## Stage 4 · P1 · SQL execution and transactions — #394

**GitHub tracker:** [#394 Match SQL execution and transaction behavior](https://github.com/kalamdb/KalamDB/issues/394) — open.

**Start:** #392 G1, #400 G2 and #393 G3; implement against completed type families rather than waiting for every PG type.

**Gate:** G4: the agreed ordinary application slice matches PostgreSQL 18 results, types, SQLSTATE, permissions and committed/rolled-back state. Full epic closure covers the complete required matrix, including concurrent and failed-transaction behavior.

**Handoff:** #399/#395 can progress in parallel after G2/G3, but transaction-sensitive catalogs and protocol completion require the relevant G4 evidence. #396 consumes the integrated behavior.

**Related issues:** [#359](https://github.com/kalamdb/KalamDB/issues/359) Unify row versioning with Raft-derived VersionId; explicit stream timestamps and gap-free replay (open); [#391](https://github.com/kalamdb/KalamDB/issues/391) Block PR #390: assign each SHARED table to exactly one persisted Raft shard and VersionDomain (open).

**Outcome:** Compatible SQL produces PostgreSQL-equivalent results and state changes.

- Implement and verify functions, operators, aggregates, collation behavior, and expression evaluation.

- Match constraints, defaults, identity columns, sequences, schema resolution, and object dependencies.

- Match transaction boundaries, rollback, savepoints, isolation, locking, concurrent writes, and failed-transaction behavior.

- Inventory the remaining server behavior required by the full-compatibility goal, including views, triggers, routines, roles, privileges, and row security.

- Define how these behaviors interact with KalamDB subscriptions and user-scoped data.

**Done when:** Differential tests against the baseline PostgreSQL server match results, errors, and committed state, including concurrency and rollback scenarios.

## Stage 5A · P1 · PostgreSQL 18 catalogs — #399

**GitHub tracker:** [#399 Align pg_catalog and information_schema with PostgreSQL 18](https://github.com/kalamdb/KalamDB/issues/399) — open. This workstream owns catalog implementation; #396 owns integrated tooling and migration.

**Start:** #392 G1, #400 G2 and #393 G3. Use relevant #394 semantics for lifecycle/transaction tests. No dependency on completion of #395.

- [ ] Inventory the full PG18 catalog/helper surface and distinguish implemented metadata from empty probe schemas.

- [ ] Implement core schema/table/column providers, then type/enum/routine/dependency providers from authoritative records.

- [ ] Remove wire-owned duplicate schemas/cross-crate source inclusion and validate catalog references, privilege filtering and OID lifecycle.

**Gate:** G5: shared catalog providers expose truthful, consistent schema/relation/type/enum/routine metadata, OIDs and helpers, with visibility, DDL, rollback and restart tests. Both transports consume the same providers.

**Handoff:** Run alongside #395 once contracts are stable. #395 consumes catalog APIs; joint end-to-end validation does not create a whole-epic dependency cycle. #396 owns broader tooling/migration workflows.

Cover the complete PG18 catalog/view/helper inventory, real type/enum/routine metadata, persistent collision-safe OIDs, permissions, dependency lifecycle and both transports. Keep PostgreSQL's actual pg_namespace identifiers. The detailed coverage checklist remains in #399 and the type/routine/catalog requirements below.

## Stage 5B · P1 · PostgreSQL connections and clients — #395

**GitHub tracker:** [#395 Support PostgreSQL connections and clients](https://github.com/kalamdb/KalamDB/issues/395) — open.

**Start:** #392 G1 and #393 G3; consume #400 schema/session interfaces and #399 catalog providers as each lands. No need to wait for all G5 catalogs to start protocol work.

**Gate:** Prepared parameter/result descriptions, binding and encoding use the same type descriptors; ordinary drivers pass both simple/extended query flows and transaction/error-state checks. Final acceptance includes G5 catalog integration and relevant G4 behavior.

**Handoff:** #396 starts integrated tooling acceptance after the required protocol/catalog/execution slices pass.

**Related issues:** [#354](https://github.com/kalamdb/KalamDB/issues/354) Add PostgreSQL wire protocol support (closed); [#375](https://github.com/kalamdb/KalamDB/issues/375) Harden PostgreSQL kalam.user_id identity override against RLS impersonation (open).

Issue #354 delivered the initial pgwire scope and explicitly excluded full PostgreSQL compatibility. The new tracker covers validation and remaining compatibility gaps.

**Outcome:** Applications connect using PostgreSQL drivers and connection settings.

- Implement connection startup, authentication, TLS, session settings, cancellation, and connection termination.

- Support simple and extended query flows, prepared statements, parameter binding, portals, result metadata, command tags, and SQLSTATE errors.

- Implement COPY and other protocol capabilities recorded in the compatibility matrix.

- Validate psql and selected Node.js, Python, Rust, and JDBC drivers using pinned versions.

**Done when:** The client suite passes connection, query, prepared statement, transaction, cancellation, bulk transfer, and reconnect tests.

## Stage 6 · P1 · Tooling and migration — #396

**GitHub tracker:** [#396 Support PostgreSQL tooling and migration](https://github.com/kalamdb/KalamDB/issues/396) — open.

**Start:** #394 ordinary workflow G4, #399 G5 and #395 required protocol/client slice.

**Gate:** Pinned psql/driver/ORM/GUI workflows introspect and migrate real schemas, custom types and routines; migrated data and schema are validated against PG18.

**Handoff:** Provides integrated evidence for #397 final validation and #398 release sign-off.

**Related issues:** [#354](https://github.com/kalamdb/KalamDB/issues/354) Add PostgreSQL wire protocol support (closed); [#50](https://github.com/kalamdb/KalamDB/issues/50) SQL API: Add schema-aware array-rows response + queryRef/schemaRef caching (Kalam-link friendly) (open).

**Outcome:** Developers can inspect schemas, run migrations, and move applications to KalamDB through familiar tooling.

- Implement PostgreSQL 18 catalog compatibility through [#399 Align pg_catalog and information_schema with PostgreSQL 18](https://github.com/kalamdb/KalamDB/issues/399). Cover real schema, table, type, enum, routine, and dependency metadata plus introspection helpers and client workflows; empty startup probe schemas are insufficient.

- Validate schema introspection, type discovery, migration generation and execution, and selected ORM workflows.

- Build a repeatable PostgreSQL-to-KalamDB migration path covering schema, data, sequences, constraints, and indexes.

- Inventory extension, procedural language, backup, and replication compatibility separately so they cannot disappear behind a broad compatibility claim.

**Done when:** Representative applications migrate and run successfully; migrated data and schema are validated, and every remaining tooling or ecosystem gap is visible in the matrix.

## Continuous P0 · Final stage 7 · Compatibility verification — #397

**GitHub tracker:** [#397 Prove compatibility and protect KalamDB behavior](https://github.com/kalamdb/KalamDB/issues/397) — open.

**Start:** Start now alongside #392. Establish the harness and baselines before behavior changes; final closure requires all functional epic evidence.

**Gate:** Each implementation change brings PG18 differential coverage and migrated existing tests. Final stage 7 requires full regression, architecture/deletion review, performance budgets and an unchanged ordinary PostgreSQL developer journey on the integrated release candidate.

**Handoff:** Its harness supports every stage. Its final sign-off unlocks #398 release approval; P0 means continuous priority, not that the entire epic must close before #392 starts.

**Related issues:** [#375](https://github.com/kalamdb/KalamDB/issues/375) Harden PostgreSQL kalam.user_id identity override against RLS impersonation (open); [#373](https://github.com/kalamdb/KalamDB/issues/373) Bound authorization cache memory across principals and policy scopes (open); [#368](https://github.com/kalamdb/KalamDB/issues/368) SDK: Add durable live-event acknowledgements and bounded flow control to TypeScript and Rust (open); [#383](https://github.com/kalamdb/KalamDB/issues/383) Stop remaining deep copies of cached manifests and rows on query/DML paths (open).

**Outcome:** Compatibility is continuously verified without breaking KalamDB capabilities.

- Run a versioned differential suite against PostgreSQL in CI and retain reproducible failure cases.

- Cover malformed SQL, hostile inputs, parameter binding, authorization, and isolation between users.

- Verify subscriptions, SDKs, CLI, and Admin UI across schema and type changes.

- Measure query latency, parsing cost, allocations, and retained memory against agreed budgets; enforce bounded caches and remove obsolete code.

**Done when:** All required compatibility and regression suites pass, performance budgets are met, and no release-blocking correctness, security, or data-loss defects remain.

## Stage 8 · P2 · Release readiness — #398

**GitHub tracker:** [#398 Make 0.8 ready for developers and production](https://github.com/kalamdb/KalamDB/issues/398) — open.

**Start:** #397 final gate, required #392–#396/#399/#400 acceptance evidence and resolved release blockers.

**Gate:** The actual release artifacts pass clean installation/onboarding, upgrade, backup/restore, restart/recovery and supported-platform checks; compatibility and breaking-change documentation match verified behavior.

**Handoff:** Publish only after the required gates pass. P2 is later delivery order, not optional scope.

**Related issues:** [#359](https://github.com/kalamdb/KalamDB/issues/359) Unify row versioning with Raft-derived VersionId; explicit stream timestamps and gap-free replay (open); [#391](https://github.com/kalamdb/KalamDB/issues/391) Block PR #390: assign each SHARED table to exactly one persisted Raft shard and VersionDomain (open); [#368](https://github.com/kalamdb/KalamDB/issues/368) SDK: Add durable live-event acknowledgements and bounded flow control to TypeScript and Rust (open).

**Outcome:** A fresh install and an upgrade both deliver a usable, recoverable release.

- Walk through a clean project from installation and initialization through kalam dev, schema creation, application startup, queries, subscriptions, and Admin UI.

- Validate restart durability, upgrade compatibility, backup and restore, and recovery from interrupted operations.

- Publish PostgreSQL compatibility documentation, migration guidance, working examples, release notes, and any breaking changes.

- Verify release artifacts, supported platforms, diagnostics, and operational instructions.

**Done when:** The complete developer journey and upgrade and recovery rehearsals pass using the actual release candidate.

## Implementation constraints and test migration

Consolidate the existing type system into one authoritative PostgreSQL-oriented logical model. Preserve useful TypeRegistry machinery, stable custom identities, revisions, field slots, and shared Arrow layouts while retiring independent KalamDataType definitions and duplicate mappings. Arrow/DataFusion and pgwire representations are derived adapters. PostgreSQL compatibility does not require PostgreSQL physical storage.

Use one PostgreSQL parse and pass the typed AST to DataFusion. Reuse its planner, optimizer, casts, and vectorized execution where semantics match. Centralize type mappings and isolate proven compatibility gaps in typed coercion or planning hooks. Avoid SQL text rewrites, duplicate registries, per-row schema construction, and unnecessary JSON/string conversions.

Validate semantic differences explicitly: FLOAT precision, NUMERIC precision and scale, JSON versus JSONB, character modifiers, temporal types, arrays, enums, and composites. Preserve current physical representations where correct; require an explicit storage migration decision where they cannot represent the required behavior.

All existing test surfaces must migrate with the syntax change: backend, CLI, pgwire, PostgreSQL extension, SDKs, Admin UI, fixtures, snapshots, seed SQL, templates, examples, and benchmarks. Preserve behavioral coverage, add negative tests for retired syntax, and keep Kalam extensions explicit. Compare behavior with PostgreSQL and test custom types through storage, schema evolution, and client round trips. Do not make the suite pass by skipping tests or weakening assertions.

These are required acceptance gates in [#392 SQL syntax](https://github.com/kalamdb/KalamDB/issues/392), [#393 data types](https://github.com/kalamdb/KalamDB/issues/393), and [#397 compatibility testing](https://github.com/kalamdb/KalamDB/issues/397). Benchmark allocations, memory, throughput, and latency before and after; the complete release-candidate test sweep and fresh-project walkthrough must pass.

DataFusion provides [AST-to-plan conversion](https://docs.rs/datafusion/55.1.0/datafusion/execution/session_state/struct.SessionState.html#method.statement_to_plan). Use the PostgreSQL [numeric](https://www.postgresql.org/docs/18/datatype-numeric.html) and [JSON](https://www.postgresql.org/docs/18/datatype-json.html) specifications to define the corresponding semantic tests.

## Required design review for one canonical type system

**Essential release gate:** Use one authoritative PostgreSQL-oriented logical type model. Arrow/DataFusion layouts and pgwire descriptors must derive from it. A new PostgreSQL enum alongside the existing KalamDataType enum is not an acceptable final design.

- [ ] Define one type contract for columns, custom-type fields, parameters, routine signatures, and results, with explicit identity and modifiers.

- [ ] Use one catalog/registry for built-ins, custom types, and Kalam extensions. Preserve stable identities, revisions, field slots, and shared layouts; retire duplicate logical definitions and redundant column fields.

- [ ] Document exact module ownership and an acyclic dependency diagram for the type core/catalog, SQL resolution, DataFusion adapter, storage codec, and pgwire adapter. Keep transport dependencies outside the core and each model in its own file.

- [ ] Share type resolution between prepared descriptions, parameter binding, and row encoding. Delete duplicate mapping tables; do not silently advertise unknown types as TEXT.

- [ ] Resolve and cache immutable layouts once per applicable schema/plan revision. Keep Arrow vectorized execution, bounded retention, and correct invalidation; avoid per-row type lookup and unnecessary conversions.

- [ ] Verify logical identity through expressions, casts, joins, aggregates, storage, and wire results. Verify the pinned DataFusion APIs and PostgreSQL semantics rather than assuming extension metadata supplies them.

- [ ] Before broad migration, review the architecture decision and prove numeric(12,2), jsonb, timestamptz, a named enum, and a composite array through planning, hot storage, Parquet, prepared statements, and text/binary results.

- [ ] Require PostgreSQL-aligned test migration, architecture regression checks, performance comparisons, and a completed inventory of deleted definitions, mappings, fallbacks, and caches. Temporary shims need an owner and removal milestone.

The detailed design and review checklist lives in [#393](https://github.com/kalamdb/KalamDB/issues/393), with integration gates in [#392](https://github.com/kalamdb/KalamDB/issues/392), [#395](https://github.com/kalamdb/KalamDB/issues/395), and [#397](https://github.com/kalamdb/KalamDB/issues/397). Design blockers must be resolved before broad migration; all completion evidence must be attached before closing the epic.

## PostgreSQL 18 types enums routines and catalogs

**Required scope:** All canonical data types must work consistently in every PostgreSQL-legal declaration context, including table columns, CREATE TYPE composite fields, arrays, enums/domains, function and procedure arguments, defaults, output arguments, and return values. The same resolver and registry must drive all of these surfaces.

- [ ] Align CREATE/ALTER/DROP TYPE and enum operations, domains, CREATE FUNCTION, and CREATE PROCEDURE with PostgreSQL 18. Inventory remaining type forms and routine attributes explicitly.

- [ ] Preserve PostgreSQL context rules. Shared type identity does not mean identical modifier enforcement everywhere: CREATE FUNCTION discards parenthesized type modifiers.

- [ ] Match function/procedure distinctions, argument modes/defaults, overloads, return forms, calls, body quoting, dependency handling, and supported language execution.

- [ ] Audit and migrate custom syntax, including PROCEDURE ... RETURNS and type/argument extensions. Keep any retained Kalam syntax explicit and documented.

- [ ] Use a type-family by legal-context test matrix. Exercise declarations, calls, casts, storage, SDK contracts, prepared descriptions, and text/binary results; update existing fixtures, generators, examples, and Admin UI tests.

- [ ] Implement real, consistent pg_catalog and information_schema projections from the authoritative catalog. Include schema/table/column metadata, pg_type, pg_enum, pg_proc, arrays/composites/domains, constraints, dependencies, visibility, and introspection helpers.

- [ ] Audit the full PG18 catalog/view/helper inventory and track each gap. Replace empty probe schemas when implemented objects require real rows; validate OIDs and references through DDL, rollback, restart, and privilege changes.

- [ ] Verify psql and pinned driver/ORM/GUI introspection against the same fixtures and a PostgreSQL 18 reference server.

Detailed requirements: [#393 unified type and routine design](https://github.com/kalamdb/KalamDB/issues/393), [#392 syntax](https://github.com/kalamdb/KalamDB/issues/392), [#399 PostgreSQL 18 catalog compatibility](https://github.com/kalamdb/KalamDB/issues/399), and [#397 validation](https://github.com/kalamdb/KalamDB/issues/397).

Baseline references: PostgreSQL 18 [CREATE TYPE](https://www.postgresql.org/docs/18/sql-createtype.html), [CREATE FUNCTION](https://www.postgresql.org/docs/18/sql-createfunction.html), [CREATE PROCEDURE](https://www.postgresql.org/docs/18/sql-createprocedure.html), [system catalogs](https://www.postgresql.org/docs/18/catalogs.html), and [information schema](https://www.postgresql.org/docs/18/information-schema.html).

## Gate handoffs

1. G1 (#392): Finish the shared PostgreSQL parse and DataFusion planning boundary before broad schema/type migration. Start #397's baseline and differential harness immediately.

2. G2 (#400), then G3 (#393): Stabilize SchemaId/resolution and migration contracts, then review and prove the one-type-system vertical slice. Schema/type audits may overlap; downstream production migration uses the accepted contracts.

3. G4 (#394), with G5 (#399) and protocol (#395): Deliver semantic behavior, catalog providers and protocol integration in tested feature slices. Catalog/wire work can overlap after G2/G3; final lifecycle and transaction checks use G4.

4. Stage 6 (#396), then final stage 7 (#397): Verify tooling/migrations and the unchanged PostgreSQL application fixture, then the full existing regression suite, architecture cleanup and performance budgets.

5. Stage 8 (#398): Sign off actual release artifacts after clean onboarding, upgrades, backup/restore, restart and recovery. Unresolved compatibility gaps require an explicit product decision and accurate release wording.

## 0.8 release checklist

- [ ] Every epic has an owner, implementation tasks, and attached acceptance evidence.

- [ ] PostgreSQL baseline and full compatibility scope are agreed.

- [ ] SQL syntax, data types, execution behavior, protocol, and tooling satisfy the compatibility matrix.

- [ ] Any proposed exclusions have an explicit product decision and accurate release wording.

- [ ] Differential tests and KalamDB regression tests pass on the release candidate.

- [ ] No release-blocking correctness, security, or data-loss issues remain.

- [ ] Performance and memory budgets pass.

- [ ] Fresh-project onboarding, upgrade, backup, restore, and restart checks pass.

- [ ] Documentation, migration examples, and release artifacts are ready.





## PostgreSQL reference baseline

Use the PostgreSQL 18 [SQL language documentation](https://www.postgresql.org/docs/18/sql.html), [data type catalog](https://www.postgresql.org/docs/18/datatype.html), and [frontend and backend protocol specification](https://www.postgresql.org/docs/18/protocol.html) to build the compatibility inventory and acceptance cases.

