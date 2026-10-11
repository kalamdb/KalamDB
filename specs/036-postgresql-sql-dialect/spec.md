# Feature Specification: PostgreSQL-Only SQL Language

**Feature Branch**: `036-postgresql-sql-dialect`

**Created**: 2026-09-20

**Status**: Design revised; implementation pending

**Revised**: 2026-10-04

**Compatibility target**: PostgreSQL 18 SQL syntax for the supported execution surface, plus explicitly documented Kalam extensions. See [capability matrix](contracts/capability-matrix.md). Grammar acceptance does not imply that every PostgreSQL engine feature is implemented.

**Input**: User description: "Convert KalamDB SQL parsing to sqlparser-rs APIs and PostgreSQL syntax only. Stop mixing DuckDB and PostgreSQL dialects. Remove homemade custom SQL parsers where PostgreSQL already defines the statement. Make KalamDB compatible with PostgreSQL clients and grammar. Produce a complete spec for implementation."

## Product Principles

These rules are non-negotiable for this feature:

1. **PostgreSQL is the public SQL language.** HTTP SQL, PostgreSQL wire, the CLI, and the extension bridge accept the same grammar.
2. **There is one dialect, not two.** Query planning must not require a second engine dialect to understand statements that classification already accepted as PostgreSQL.
3. **Standard PostgreSQL statements are PostgreSQL statements.** Schema, table, view, index, type, policy, comment, grant/revoke, call, search_path, and transaction control are not a second homemade language.
4. **Kalam extensions must not steal PostgreSQL prefixes.** A Kalam-only keyword sequence is valid only when it cannot be a PostgreSQL command.
5. **Kalam-only operations remain Kalam-only.** Storage, cluster, subscribe, topic, and schedule commands stay available, but they are extensions of PostgreSQL parsing, not a parallel SQL dialect.
6. **Parse once.** Classification, validation, and execution share one parse of each statement.
7. **Prefer PostgreSQL-shaped syntax when Kalam already has an equivalent.** Aliases listed in this feature remain supported until a separately announced removal; the documented form is the PostgreSQL form.
8. **Do not rename every Kalam operations keyword in this feature.** Compatibility with PostgreSQL standard statements comes first. Mapping `STORAGE FLUSH` to `VACUUM` / `CHECKPOINT` is out of scope.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Connect a PostgreSQL Client and Run Standard DDL (Priority: P1)

A developer using `psql`, JDBC, or a GUI connects to KalamDB and runs ordinary PostgreSQL statements: `CREATE SCHEMA`, `CREATE TABLE`, `CREATE INDEX`, `CREATE VIEW`, `COMMENT ON`, `CREATE POLICY`, `GRANT EXECUTE`, `CALL`, `SET search_path`, `BEGIN` / `COMMIT`. Those statements succeed or fail as PostgreSQL-shaped SQL, not as unrecognized “extension commands”.

**Why this priority**: Wire and GUI access already exist. The remaining friction is a split language: some statements are PostgreSQL, others are string-matched Kalam dialects that collide with PostgreSQL (`CREATE USER TABLE` vs `CREATE USER`).

**Independent Test**: On a fresh namespace, run the PostgreSQL forms of schema/table/policy/comment/grant/call without any `CREATE USER TABLE` / `CREATE NAMESPACE` prefixes. Confirm catalog state and a subsequent `SELECT` succeed.

**Acceptance Scenarios**:

1. **Given** an authenticated PostgreSQL-compatible session, **When** the client runs `CREATE SCHEMA IF NOT EXISTS app`, **Then** a namespace named `app` exists and a second identical statement with `IF NOT EXISTS` does not fail.
2. **Given** schema `app`, **When** the client runs `CREATE TABLE app.items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)`, **Then** the table is created and `INSERT` / `SELECT` work.
3. **Given** that table, **When** the client runs PostgreSQL `CREATE POLICY`, `COMMENT ON`, `GRANT EXECUTE` (on a procedure), and `SET search_path TO app`, **Then** each statement is accepted as that PostgreSQL command, not rejected as an unknown Kalam extension.
4. **Given** `CREATE USER alice WITH PASSWORD 'secret'`, **When** the statement is executed, **Then** it creates a login user and is **not** interpreted as a table-creation command.

---

### User Story 2 - Create Kalam Table Kinds Without Colliding With PostgreSQL (Priority: P1)

An application developer still needs user, shared, and stream tables. They specify table kind with PostgreSQL-compatible table options, not a `CREATE USER TABLE` prefix that makes `CREATE USER` impossible.

**Why this priority**: `CREATE USER TABLE` is the highest-impact incompatibility with PostgreSQL. Table kinds must remain expressible.

**Independent Test**: Create one user table, one shared table, and one stream table using `CREATE TABLE ... WITH (TYPE = ...)`. Confirm `CREATE USER name WITH PASSWORD ...` still creates a user.

**Acceptance Scenarios**:

1. **Given** schema `app`, **When** the client runs `CREATE TABLE app.inbox (id BIGINT PRIMARY KEY) WITH (TYPE = 'USER')`, **Then** the table is a user table.
2. **Given** the same schema, **When** the client runs `CREATE TABLE app.events (id BIGINT PRIMARY KEY) WITH (TYPE = 'SHARED')`, **Then** the table is a shared table.
3. **Given** stream options required by current product rules, **When** the client runs `CREATE TABLE app.clicks (...) WITH (TYPE = 'STREAM', ...)`, **Then** the table is a stream table.
4. **Given** `CREATE USER TABLE app.legacy (...)`, **When** the statement is submitted, **Then** the server rejects it with a migration message pointing to `CREATE TABLE ... WITH (TYPE = 'USER')`, and `CREATE USER` remains available.
5. **Given** existing `CREATE SHARED TABLE` / `CREATE STREAM TABLE` aliases, **When** they are submitted during this feature, **Then** they still work as aliases of `WITH (TYPE = 'SHARED'|'STREAM')` throughout this feature.

---

### User Story 3 - One Language From Classification Through Query Execution (Priority: P1)

An operator runs the same `SELECT` (including PostgreSQL `UNNEST` catalog probes used by GUI tools) through HTTP SQL and through PostgreSQL wire. Classification and execution agree on the dialect. The server does not parse the statement as PostgreSQL and then re-parse it as a second dialect to plan it.

**Why this priority**: Today classification leans PostgreSQL while execution planning uses another dialect for array lambdas. That split is the reason catalog `UNNEST` has to be rewritten and why `->` is ambiguous.

**Independent Test**: Run a GUI-style `JOIN UNNEST(...)` catalog probe and a normal `SELECT` with PostgreSQL `->` JSON operators (if already supported) on both HTTP SQL and wire. Confirm they plan without requiring a second public dialect.

**Acceptance Scenarios**:

1. **Given** any supported entry point, **When** a statement is classified and then executed, **Then** both steps use the same PostgreSQL language rules.
2. **Given** a PostgreSQL `SELECT` that today’s GUI catalog shims already serve, **When** it is executed after this feature, **Then** it returns the same visible rows and column metadata for the unchanged query text; a newly introduced error or replacement query does not satisfy compatibility.
3. **Given** a query that uses DuckDB-only lambda syntax such as `array_transform(arr, x -> x * 10)`, **When** it is submitted, **Then** the server rejects it with a clear “unsupported PostgreSQL syntax” (or equivalent) message rather than planning it under a second dialect.
4. **Given** PostgreSQL array/unnest forms that DataFusion can execute, **When** they are submitted, **Then** they run without enabling a second dialect.

---

### User Story 4 - Kalam Extensions Parse as PostgreSQL Extensions (Priority: P2)

An operator still runs Kalam-only commands (`CREATE STORAGE`, `STORAGE FLUSH`, `SUBSCRIBE TO`, topic/cluster/schedule commands). They keep working, but they are recognized as extensions in a PostgreSQL session: quoted identifiers, comments, and string literals follow PostgreSQL rules, and they do not require a separate uppercase string matcher.

**Why this priority**: Extensions are real product surface. They must stop being a second lexer, but this feature does not rename them to `VACUUM` / `LISTEN`.

**Independent Test**: Run `CREATE STORAGE`, `STORAGE FLUSH TABLE`, `SUBSCRIBE TO`, and one topic command with PostgreSQL quoting (`"MixedCase"`, `'string'`, `-- comment`) and confirm they parse and execute.

**Acceptance Scenarios**:

1. **Given** an admin session, **When** they run a storage or cluster command that includes a `--` line comment and a quoted identifier, **Then** the comment is ignored and the identifier is preserved with PostgreSQL quoting rules.
2. **Given** `SUBSCRIBE TO app.messages WHERE user_id = CURRENT_USER`, **When** it is parsed, **Then** the `WHERE` expression is a PostgreSQL expression, not a raw substring split.
3. **Given** an unknown statement that is neither PostgreSQL nor a documented Kalam extension, **When** it is submitted, **Then** the error is a single syntax/unknown-command error, not a chain of “extension parse failed” plus “SQL parse failed”.

---

### User Story 5 - Migrate Existing Kalam SQL With Explicit Aliases (Priority: P2)

Teams with scripts that use `CREATE NAMESPACE`, `USE NAMESPACE`, and `CREATE SHARED TABLE` can keep those aliases during this feature. The documented canonical forms are PostgreSQL `CREATE SCHEMA`, `SET search_path`, and `CREATE TABLE ... WITH (TYPE = ...)`.

**Why this priority**: Compatibility for existing tests and apps, without leaving colliding prefixes.

**Independent Test**: Run the alias forms and the PostgreSQL forms against the same catalog objects and confirm they are equivalent.

**Acceptance Scenarios**:

1. **Given** `CREATE NAMESPACE app` and `CREATE SCHEMA app`, **When** each is used on a fresh database, **Then** both create the same kind of namespace/schema object.
2. **Given** `USE app`, `USE NAMESPACE app`, `SET NAMESPACE app`, and `SET search_path TO app`, **When** a subsequent unqualified `SELECT` runs, **Then** it resolves objects in `app`.
3. **Given** documentation and CLI help, **When** a developer looks up how to create a schema or a user, **Then** the examples use PostgreSQL syntax, with aliases listed as compatibility forms.

---

### User Story 6 - Adding a New SQL Command Follows One Path (Priority: P3)

A KalamDB engineer adds a new Kalam-only command. They extend the shared PostgreSQL-based language layer and map a typed result into the existing statement kinds. They do not add a new whitespace tokenizer or `starts_with("CREATE FOO")` parser.

**Why this priority**: Prevents the homemade parser set from growing back after this cleanup.

**Independent Test**: The contributor guide describes one parse path; a review checklist rejects a new string-prefix parser for standard SQL.

**Acceptance Scenarios**:

1. **Given** the updated contributor guide, **When** an engineer adds a Kalam-only statement, **Then** the documented steps use the shared PostgreSQL parse path and its typed custom-command parser in the dialect crate.
2. **Given** a proposed change that reimplements `CALL`, `CREATE SCHEMA`, or `GRANT` with string splitting, **When** it is reviewed against this spec, **Then** it is out of bounds.

---

### Edge Cases

- `CREATE USER name ...` vs any table-kind prefix that begins with `CREATE USER`.
- `DROP USER name` vs `DROP USER TABLE` (reject with a migration error directing the user to `DROP TABLE`).
- `CREATE STREAM TABLE` vs any future `CREATE STREAM` that is not a table.
- Dollar-quoted bodies on `CREATE PROCEDURE ... AS $$ ... $$` (PostgreSQL quoting, not T-SQL `BEGIN...END`).
- `CURRENT_USER` vs `CURRENT_USER()` in defaults and `WHERE` clauses (PostgreSQL keyword form is canonical).
- Multiple statements in one string (`;` separated) still split under PostgreSQL lexer rules, including dollar quotes.
- GUI `LIMIT`/`OFFSET` appended to `CALL` is rejected consistently as unsupported Kalam call syntax; no silent suffix stripping. JDBC escape calls are a documented compatibility adapter using the same token stream, never a regex rewrite.
- Invalid SQL produces one error with position/token context from the PostgreSQL parser when the statement is attempted as SQL, not a generic “unknown extension command” that hides the real syntax problem.
- Unqualified `pg_*` catalog probes continue to work through existing shims; this feature does not remove `pg_catalog`.
- Batch HTTP SQL and wire simple-query both see the same grammar.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Every SQL entry point MUST accept the same PostgreSQL-compatible grammar for a given statement text.
- **FR-002**: Statement classification and query execution MUST use the same PostgreSQL language rules for the same statement.
- **FR-003**: The server MUST parse standard PostgreSQL statements with the shared SQL parser rather than statement-specific string or token homemade parsers. This includes at least: `CREATE SCHEMA`, `CREATE TABLE`, `ALTER TABLE`, `DROP TABLE`, `CREATE VIEW`, `CREATE INDEX`, `CREATE TYPE` / `ALTER TYPE` / `DROP TYPE` (PostgreSQL enum/composite forms), `CREATE POLICY` / `ALTER POLICY` / `DROP POLICY`, `COMMENT ON`, `GRANT` / `REVOKE` (including `EXECUTE ON PROCEDURE`), `CALL`, `SET` / `RESET search_path`, `BEGIN` / `COMMIT` / `ROLLBACK`, and `EXPLAIN`. `USE` and `DESCRIBE` are Kalam compatibility forms, not PostgreSQL standard statements. Where the pinned upstream parser cannot model a required PostgreSQL form, the shared token parser may produce a typed PostgreSQL-compatibility node; such gaps require regression fixtures.
- **FR-004**: `CREATE USER` / `ALTER USER` / `DROP USER` MUST be user-identity commands. They MUST NOT be parsed as table DDL.
- **FR-005**: User, shared, and stream tables MUST be created with `CREATE TABLE ... WITH (TYPE = 'USER'|'SHARED'|'STREAM')` (plus existing table options). `CREATE USER TABLE` MUST be rejected with a migration error.
- **FR-006**: `CREATE SHARED TABLE` and `CREATE STREAM TABLE` MUST remain aliases of the `WITH (TYPE = ...)` form in this feature.
- **FR-007**: `CREATE SCHEMA` is the canonical namespace-creation statement. `CREATE NAMESPACE` MUST remain an alias that creates the same object.
- **FR-008**: `SET search_path` / `RESET search_path` are canonical session-schema statements. `USE`, `USE NAMESPACE`, and `SET NAMESPACE` MUST remain aliases that set the same default schema.
- **FR-009**: `CREATE PROCEDURE` / `DROP PROCEDURE` MUST follow PostgreSQL-style procedure syntax used by Kalam functions (`LANGUAGE`, `AS $$ ... $$`, `SECURITY INVOKER|DEFINER`), not a second T-SQL procedure grammar.
- **FR-010**: Topic `CREATE TRIGGER ... ON TOPIC ... EXECUTE PROCEDURE` MUST remain a Kalam extension. It MUST NOT be parsed as a PostgreSQL table trigger.
- **FR-011**: Kalam-only commands (storage, cluster, subscribe, topic, schedule, backup/restore, export, kill job / kill live query) MUST parse with PostgreSQL lexer rules (comments, quoted identifiers, string literals) through the shared parse path.
- **FR-012**: DuckDB-only public syntax, including lambda expressions of the form `x -> expr` used with `array_transform` / `array_filter` / `array_any_match`, MUST be rejected on all entry points.
- **FR-013**: PostgreSQL `UNNEST` in catalog/GUI queries that already work MUST continue to work after the second dialect is removed, natively or via a tested AST/planner adaptation; query text must not be rewritten and parsed again.
- **FR-014**: Each successfully parsed statement MUST have one backend parse from its batch's single tokenization for classification, authorization, metadata inference, and planning. Prepared binds/executions and retries MUST reuse its immutable parsed representation. AST transformations and replanning are allowed; AST-to-SQL-to-AST round trips are not. Nested stored SQL has its own parse lifetime keyed by its definition version. Non-SQL procedure bodies remain opaque language payloads.
- **FR-015**: Malformed standard SQL MUST surface as SQL syntax errors; valid but unsupported features MUST receive a distinct unsupported-feature error. Kalam extension errors MUST name the extension command. The server MUST NOT try every extension parser and then fail with a combined leftover message.
- **FR-016**: Authorization checks that already exist (admin for storage/namespace/cluster, policy roles, etc.) MUST still run, using the parsed statement kind, not a second independent word list as the source of truth.
- **FR-017**: Active tests, examples, CLI schema generation, UI completion, SDK SQL producers, and docs that demonstrate removed syntax MUST be updated in this feature. Canonical examples MUST use PostgreSQL syntax. Negative migration fixtures and historical release/spec records may retain removed forms with an explicit purpose; generated outputs must be regenerated from their sources.
- **FR-018**: The contributor guide for adding SQL statements MUST describe the single PostgreSQL parse path and MUST NOT instruct engineers to add `starts_with` / whitespace token parsers for standard SQL.
- **FR-019**: Default `CREATE TABLE` kind when `TYPE` is omitted MUST stay the current product default (shared). This feature MUST NOT silently change table-kind defaults.
- **FR-020**: Wire-protocol and HTTP SQL MUST remain able to run the PostgreSQL statements in User Story 1 after this change without a client-side dialect switch.

- **FR-021**: The capability matrix MUST identify PostgreSQL 18 forms that execute, parse but are unsupported, require a pinned-parser compatibility node, or are Kalam extensions. Unsupported upstream syntax MUST be reported as a documented parser limitation, never claimed as full PostgreSQL support.
- **FR-022**: Standard SQL and Kalam commands MUST share PostgreSQL lexical behavior: lower-case folding of unquoted identifiers, preservation of quoted identifiers, escaped quotes, tagged dollar strings, escape strings, nested block comments, and statement boundaries. Quoted names MUST resolve consistently in DDL, DML, catalogs, and authorization.
- **FR-023**: Session search paths MUST preserve ordered schema entries, resolve `"$user"` to the authenticated user's schema when present, and honor implicit `pg_catalog` lookup. `SET SESSION`, transaction-scoped `SET LOCAL`, and `RESET` MUST behave consistently through HTTP sessions and wire. Stateless HTTP requests use a request-local context. Unsupported temporary schemas and transaction options MUST fail explicitly, not silently no-op.
- **FR-024**: HTTP single/batch SQL, wire simple and extended queries, WebSocket subscription SQL, nested SQL in views/procedures/schedules, and SQL emitted by CLI/SDK/extension adapters MUST use the same backend parse contract. Transport-specific command availability may differ, but cannot change grammar. Typed FDW requests that contain no SQL are not required to serialize themselves into SQL.
- **FR-025**: No public SQL/session/config override may select a different parser dialect. Non-PostgreSQL syntax outside the explicit extension/alias inventory MUST be rejected, including DuckDB lambda execution. Valid PostgreSQL JSON operators MUST not be banned by token spelling.
- **FR-026**: Parameter/result metadata MUST derive from the shared AST and permission-aware planning context. Binds remain typed and request-local; prepared execution MUST preserve role, tenant, search-path, and schema-version isolation without reparsing on cache hits.
- **FR-027**: Existing stored SQL using removed forms MUST be detectable before upgrade and fail with object identity and a migration instruction if encountered after upgrade. Provide an explicit, reviewable migration procedure; never silently rewrite persisted user definitions or retain a legacy runtime parser.
- **FR-028**: SQL errors MUST have stable categories and original source positions where available. Wire maps syntax/unsupported/authorization errors to SQLSTATE 42601/0A000/42501; HTTP and WebSocket preserve equivalent categories without exposing protected schema or credential details.
- **FR-029**: The feature MUST ship positive and negative corpus tests, unchanged GUI-query regressions, role/tenant authorization regressions, parse-count assertions, and cross-entry-point tests. Grammar tests MUST preserve recursion/resource limits and include malformed/oversized/deep input cases.
- **FR-030**: Canonical SQL references, contributor and architecture docs, canonical kalamdb-skills content and generated mirrors, and docs/tests for affected SDKs MUST ship with the migration. External documentation updates that cannot be completed or validated locally MUST be reported as outstanding delivery work.

- **FR-031**: Each converted parser path MUST remove its obsolete dispatcher, lexer, regex rewrite, adapter, dead model/export and superseded tests in the same implementation slice once callers have moved. Maintain a deletion ledger with old symbol/path, replacement and remaining callers; zero obsolete production parser paths or temporary dual-parser flags may remain at release. Remove dependencies/features only when no retained code needs them.
- **FR-032**: Custom statements MUST reuse sqlparser-rs identifier/object-name, type, expression, list and token helpers; AST traversal MUST reuse its visitor APIs where suitable. Local grammar code is limited to Kalam productions and demonstrated gaps in the pinned PostgreSQL parser. Every gap records upstream evidence, a regression fixture and the minimal extension; no copied upstream grammar, second Pratt parser, custom lexer or wholesale mirrored AST.
- **FR-033**: Parser integration MUST use safe Rust with owned immutable ASTs, bounded source retention and bounded parse/cache admission. No new unsafe, lifetime extension, leaked allocation, global interning of untrusted SQL, reference cycles, or retained token streams. Preserve upstream recursive protection and enforce request/depth/statement and memory-admission limits on all SQL surfaces, including custom commands and nested SQL. Errors, cancellation, eviction and disconnect MUST release owned resources; diagnostics must not leak secrets.
- **FR-034**: Untrusted values MUST remain typed bindings through planning/execution; object names MUST remain typed identifier components resolved and authorized separately. No user-derived value, identifier, custom option or stored-definition content may be concatenated into privileged SQL. Explicit SQL input uses its caller's permission context; SECURITY DEFINER execution retains its explicit authorization boundary. Single-command APIs reject trailing statements; multi-command APIs authorize every parsed statement. sqlparser acceptance alone is never a security check.
- **FR-035**: Parse-once instrumentation MUST cover every standard, PostgreSQL-compatibility and Kalam command family, including failed input (at most one attempt; pre-admission rejection performs zero parsing), batch members, embedded predicates, subscriptions, procedure host SQL, schedules and stored definitions. Custom handlers consume typed payloads and existing Expr/Query nodes, never re-tokenize raw options or remainder text. One submitted batch is tokenized once; each statement or genuinely separate nested SQL definition is parsed once per admitted lifetime, with zero reparses during classification, authorization, metadata, bind, execute or retry.
- **FR-036**: Measure cold parse/classify/plan and warm reuse for standard and custom SQL under equal-work concurrency. Report allocations/bytes, retained source/AST/cache bytes, peak admitted memory, AST clones and p50/p95 latency in addition to runtime seconds. Reuse existing caches and resource managers; introduce no speculative buffer pools, unsafe zero-copy tricks or additional caching layer. Enforce the measurable gates in contracts/efficiency-security-cleanup.md.

### Key Entities

- **SQL statement text**: The original command submitted on any entry point.
- **PostgreSQL parse tree**: The typed representation of standard SQL, required PostgreSQL compatibility nodes, or Kalam extensions.
- **Kalam statement kind**: The product-level classified command (create table, create user, flush storage, subscribe, …) derived from the parse tree.
- **Schema/namespace**: PostgreSQL schema; Kalam namespace. Same object; two names.
- **Table kind**: User, shared, or stream. A table option, not a `CREATE USER` prefix.
- **Kalam extension command**: A documented non-PostgreSQL statement recognized by token lookahead on the shared parser, with explicit collision rules and no parse-failure fallback chain.
- **Compatibility alias**: An old Kalam spelling that maps onto a canonical PostgreSQL statement (`CREATE NAMESPACE` → `CREATE SCHEMA`).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A PostgreSQL-compatible client can complete this script without Kalam-only DDL prefixes: `CREATE SCHEMA`, `CREATE TABLE` with columns, `INSERT`, `SELECT`, `CREATE POLICY`, `COMMENT ON`, `SET search_path`, `BEGIN`/`COMMIT`.
- **SC-002**: `CREATE USER <name> WITH PASSWORD '<pw>'` creates a user in 100% of trials and never creates a table.
- **SC-003**: 100% of active in-repo examples and SQL producers that previously used `CREATE USER TABLE` are rewritten to `CREATE TABLE ... WITH (TYPE = 'USER')` or an equivalent non-colliding form.
- **SC-004**: Submitting `SELECT array_transform(ARRAY[1,2,3], x -> x * 10)` fails on HTTP SQL and on wire with a clear unsupported-syntax error in 100% of trials.
- **SC-005**: GUI-style catalog probes that pass on the current server before this feature still pass after it (unchanged statements, same visible rows and column metadata; deterministic ordering when guaranteed by the query).
- **SC-006**: Dialect crate tests for `CALL`, `CREATE SCHEMA`, `GRANT EXECUTE`, `COMMENT ON`, `CREATE POLICY`, `SET search_path`, and `CREATE TABLE` run as one suite and pass without statement-specific string lexers for those commands.
- **SC-007**: Adding a new standard-SQL handler does not require a new `trim().to_uppercase().starts_with` parser; the contributor guide matches the implemented path.
- **SC-008**: No user-visible configuration or session setting selects a second SQL dialect for planning versus classification.
- **SC-009**: Median complete parse/classify/authorize/plan latency for the fixed SELECT/DML/catalog corpus and warm prepared-execute latency MUST each remain within +10% of their respective pre-change baselines on the same dev-profile environment. Record parse-only time, allocation counts/bytes, cache behavior, and each relevant runtime in seconds separately; never compare full parsing to classification alone. No implicit PR-level exception waives this gate.

- **SC-010**: All matrix rows have executable fixture IDs and an observed result before release; required rows pass and unsupported rows return their documented category on every applicable surface.
- **SC-011**: Instrumented tests observe one backend parse per successfully parsed submitted statement (at most one attempt for rejected input), zero additional parses for prepared binds/cache hits/retries, and one parse per nested SQL definition version while cached.
- **SC-012**: Role/tenant/search-path/cache-isolation and recursion-limit tests pass across supported entry points without changing authorization outcomes.
- **SC-013**: Upgrade fixtures detect removed syntax in stored definitions, identify the affected object, and prove that explicitly migrated definitions execute after restart.

- **SC-014**: The deletion ledger has zero obsolete production callers, the parser-boundary guard passes, and every retained compatibility implementation has a pinned-upstream gap or explicit Kalam grammar justification. Unused parser-only dependencies/features are removed without weakening recursion protection.
- **SC-015**: At least 10,000 mixed prepare/bind/drop/error/cancel lifecycle iterations satisfy configured admission/cache bounds; live parser-owned objects and retained source bytes return to the expected bounded cache state after cleanup. Warm reuse performs zero tokenization, source copies or full AST clones. Parser integration contains no new unsafe/lifetime leaks.
- **SC-016**: Injection fixtures across SQL values, identifiers, custom command options, stored SQL, batch boundaries and privileged execution preserve values as data, cause no unintended side effects or cross-role/tenant access, and do not expose credentials in application or dependency logs.
- **SC-017**: Every inventoried custom-command family has a passing one-tokenization/one-parse test plus error and concurrent-isolation cases. The same-work standard/custom corpus meets p50 and p95 latency limits of +10% and allocation-byte/count limits of +10% versus its pre-change baseline; memory/admission gates pass independently of latency.

## Assumptions

- PostgreSQL wire (033) and schema-first functions (034) remain; this feature unifies the language layer and adds ordered search-path state to the existing session/transaction owners without replacing transaction architecture.
- `CREATE TABLE ... WITH (TYPE = 'USER'|'SHARED'|'STREAM')` already exists and is the canonical table-kind syntax.
- Default table kind for bare `CREATE TABLE` stays **shared**, matching current parser behavior.
- `CREATE NAMESPACE` and `USE NAMESPACE` stay as aliases in this feature; they are not deleted yet.
- `CREATE SHARED TABLE` / `CREATE STREAM TABLE` stay as aliases; `CREATE USER TABLE` is removed because it collides with PostgreSQL `CREATE USER`.
- Kalam extension keywords (`STORAGE`, `CLUSTER`, `SUBSCRIBE`, `TOPIC`, `SCHEDULE`) are not renamed to PostgreSQL names in this feature.
- DuckDB-style SQL lambdas were a DataFusion-planning convenience, not a documented Kalam product dialect. Removing them is an accepted breaking change.
- `pg_catalog` shims stay; this is not a catalog-rewrite feature.
- Functions `CREATE PROCEDURE ... AS $$ $$` stays the 034 contract; only the parser implementation changes to PostgreSQL quoting/procedure shape.
- sqlparser 0.62 remains the parser library; Kalam does not fork it to add `Statement::CreateStorage`.
- Contributor docs in `docs/development/how-to-add-sql-statement.md` and `docs/reference/sql.md` are updated in this feature. Out-of-repo canonical skills and affected SDK docs are delivery requirements; unavailable paths remain explicit outstanding work.

## Scope Boundaries

### In scope

- One PostgreSQL-based public SQL language on all SQL entry points, with the bounded execution surface in contracts/capability-matrix.md.
- Remove homemade parsers for statements PostgreSQL already defines (see FR-003).
- Shared token-parser handling of Kalam extensions and of PostgreSQL statements that the upstream parser cannot faithfully represent (`CREATE USER`, `CREATE PROCEDURE`).
- Reject `CREATE USER TABLE` and DuckDB-only lambdas.
- Update tests, SQL reference, and the “add a SQL statement” guide.

### Out of scope

- Implementing the full PostgreSQL language (PL/pgSQL, foreign data wrappers, partitioned tables as in PostgreSQL, replication slots, etc.).
- Renaming Kalam operations to PostgreSQL names (`STORAGE FLUSH` → `VACUUM`/`CHECKPOINT`, `SUBSCRIBE TO` → `LISTEN`).
- Changing default `CREATE TABLE` kind from shared to user.
- Removing required `pg_catalog` shims. Compatibility transformations must operate on ASTs/plans and retain the unchanged client-query contract.
- New product commands.
- Forking or upgrading sqlparser except as needed for PostgreSQL parsing bugs discovered during implementation.

## Dependencies

- 033 unified backend / PostgreSQL wire (entry points and client catalog).
- 034 schema-first functions (`CREATE PROCEDURE` / `CALL` / `GRANT EXECUTE` contracts).
- 024 topic pub/sub and trigger-on-topic syntax.
- Existing `CREATE TABLE ... WITH (TYPE = ...)` options.

## Additional acceptance scenarios

- **US1**: Two schemas containing the same table prove ordered lookup; `"$user"`, quoted mixed-case names, SET LOCAL rollback, RESET, and denied system access behave consistently in stateful HTTP and wire sessions.
- **US3**: The shared corpus covers casts, arrays, JSON operator parsing, CTEs, joins, windows, ON CONFLICT, RETURNING, prepared `$1` binds, malformed syntax, unsupported features, and attempted dialect switches. Repeated binds and cache invalidation never reuse another principal's planning context.
- **US4**: Each registered extension has valid, invalid, quoted/commented, and role-denial fixtures; nested expressions consume the existing parser rather than lexing a substring.
- **US5**: CLI/UI/SDK generated SQL executes on the unified parser. Upgrade preflight reports a stored legacy definition and its replacement; an explicit migration survives restart.
- **US6**: An automated architecture guard detects direct alternative dialect construction, raw user-SQL DataFusion parse calls, and reintroduced string-prefix dispatch outside the documented compatibility boundary.

## Cleanup, reuse and security acceptance

The [efficiency, security and cleanup contract](contracts/efficiency-security-cleanup.md) is normative for all stories. Memory safety and injection resistance require implementation review and executable evidence; these planning documents do not certify existing code. Parse-once does not mean keeping every SQL string forever: a new submission or a stored-definition cache miss begins a new bounded parse lifetime.
