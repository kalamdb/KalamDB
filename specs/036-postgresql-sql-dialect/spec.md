# Feature Specification: PostgreSQL-Only SQL Language

**Feature Branch**: `036-postgresql-sql-dialect`

**Created**: 2026-09-20

**Status**: Draft

**Input**: User description: "Convert KalamDB SQL parsing to sqlparser-rs APIs and PostgreSQL syntax only. Stop mixing DuckDB and PostgreSQL dialects. Remove homemade custom SQL parsers where PostgreSQL already defines the statement. Make KalamDB compatible with PostgreSQL clients and grammar. Produce a complete spec for implementation."

## Product Principles

These rules are non-negotiable for this feature:

1. **PostgreSQL is the public SQL language.** HTTP SQL, PostgreSQL wire, the CLI, and the extension bridge accept the same grammar.
2. **There is one dialect, not two.** Query planning must not require a second engine dialect to understand statements that classification already accepted as PostgreSQL.
3. **Standard PostgreSQL statements are PostgreSQL statements.** Schema, table, view, index, type, policy, comment, grant/revoke, call, search_path, and transaction control are not a second homemade language.
4. **Kalam extensions must not steal PostgreSQL prefixes.** A Kalam-only keyword sequence is valid only when it cannot be a PostgreSQL command.
5. **Kalam-only operations remain Kalam-only.** Storage, cluster, subscribe, topic, and schedule commands stay available, but they are extensions of PostgreSQL parsing, not a parallel SQL dialect.
6. **Parse once.** Classification, validation, and execution share one parse of each statement.
7. **Prefer PostgreSQL-shaped syntax when Kalam already has an equivalent.** Aliases may exist for one migration window; the documented form is the PostgreSQL form.
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
5. **Given** existing `CREATE SHARED TABLE` / `CREATE STREAM TABLE` aliases, **When** they are submitted during this feature, **Then** they still work as aliases of `WITH (TYPE = 'SHARED'|'STREAM')` unless a later story removes them.

---

### User Story 3 - One Language From Classification Through Query Execution (Priority: P1)

An operator runs the same `SELECT` (including PostgreSQL `UNNEST` catalog probes used by GUI tools) through HTTP SQL and through PostgreSQL wire. Classification and execution agree on the dialect. The server does not parse the statement as PostgreSQL and then re-parse it as a second dialect to plan it.

**Why this priority**: Today classification leans PostgreSQL while execution planning uses another dialect for array lambdas. That split is the reason catalog `UNNEST` has to be rewritten and why `->` is ambiguous.

**Independent Test**: Run a GUI-style `JOIN UNNEST(...)` catalog probe and a normal `SELECT` with PostgreSQL `->` JSON operators (if already supported) on both HTTP SQL and wire. Confirm they plan without requiring a second public dialect.

**Acceptance Scenarios**:

1. **Given** any supported entry point, **When** a statement is classified and then executed, **Then** both steps use the same PostgreSQL language rules.
2. **Given** a PostgreSQL `SELECT` that today’s GUI catalog shims already serve, **When** it is executed after this feature, **Then** it still returns a usable result or a clear PostgreSQL-shaped error, not a “table function not found” failure caused by a second dialect.
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

Teams with scripts that use `CREATE NAMESPACE`, `USE NAMESPACE`, and `CREATE SHARED TABLE` can keep those aliases during this feature. The documented canonical forms are PostgreSQL `CREATE SCHEMA`, `SET search_path` / `USE`, and `CREATE TABLE ... WITH (TYPE = ...)`.

**Why this priority**: Compatibility for existing tests and apps, without leaving colliding prefixes.

**Independent Test**: Run the alias forms and the PostgreSQL forms against the same catalog objects and confirm they are equivalent.

**Acceptance Scenarios**:

1. **Given** `CREATE NAMESPACE app` and `CREATE SCHEMA app`, **When** each is used on a fresh database, **Then** both create the same kind of namespace/schema object.
2. **Given** `USE app`, `USE NAMESPACE app`, `SET NAMESPACE app`, and `SET search_path TO app`, **When** a subsequent unqualified `SELECT` runs, **Then** it resolves objects in `app`.
3. **Given** documentation and CLI help, **When** a developer looks up how to create a schema or a user, **Then** the examples use PostgreSQL syntax, with aliases listed as compatibility forms.

---

### User Story 6 - Adding a New SQL Command Follows One Path (Priority: P3)

A KalamDB engineer adds a new Kalam-only command. They extend the PostgreSQL dialect hook and map the result into the existing statement kinds. They do not add a new whitespace tokenizer or `starts_with("CREATE FOO")` parser.

**Why this priority**: Prevents the homemade parser set from growing back after this cleanup.

**Independent Test**: The contributor guide describes one parse path; a review checklist rejects a new string-prefix parser for standard SQL.

**Acceptance Scenarios**:

1. **Given** the updated contributor guide, **When** an engineer adds a Kalam-only statement, **Then** the documented steps use the shared PostgreSQL parse path plus a dialect extension hook.
2. **Given** a proposed change that reimplements `CALL`, `CREATE SCHEMA`, or `GRANT` with string splitting, **When** it is reviewed against this spec, **Then** it is out of bounds.

---

### Edge Cases

- `CREATE USER name ...` vs any table-kind prefix that begins with `CREATE USER`.
- `DROP USER name` vs `DROP USER TABLE` (the latter is not PostgreSQL; reject or treat as invalid `DROP USER`).
- `CREATE STREAM TABLE` vs any future `CREATE STREAM` that is not a table.
- Dollar-quoted bodies on `CREATE PROCEDURE ... AS $$ ... $$` (PostgreSQL quoting, not T-SQL `BEGIN...END`).
- `CURRENT_USER` vs `CURRENT_USER()` in defaults and `WHERE` clauses (PostgreSQL keyword form is canonical).
- Multiple statements in one string (`;` separated) still split under PostgreSQL lexer rules, including dollar quotes.
- GUI `LIMIT`/`OFFSET` appended to `CALL` remains ignored or rejected consistently; it must not require a private `CALL` string parser beyond a documented compatibility strip.
- Invalid SQL produces one error with position/token context from the PostgreSQL parser when the statement is attempted as SQL, not a generic “unknown extension command” that hides the real syntax problem.
- Unqualified `pg_*` catalog probes continue to work through existing shims; this feature does not remove `pg_catalog`.
- Batch HTTP SQL and wire simple-query both see the same grammar.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Every SQL entry point MUST accept the same PostgreSQL-compatible grammar for a given statement text.
- **FR-002**: Statement classification and query execution MUST use the same PostgreSQL language rules for the same statement.
- **FR-003**: The server MUST parse standard PostgreSQL statements with the shared SQL parser rather than statement-specific string or token homemade parsers. This includes at least: `CREATE SCHEMA`, `CREATE TABLE`, `ALTER TABLE`, `DROP TABLE`, `CREATE VIEW`, `CREATE INDEX`, `CREATE TYPE` / `ALTER TYPE` / `DROP TYPE` (PostgreSQL enum/composite forms), `CREATE POLICY` / `ALTER POLICY` / `DROP POLICY`, `COMMENT ON`, `GRANT` / `REVOKE` (including `EXECUTE ON PROCEDURE`), `CALL`, `SET` / `RESET search_path`, `USE`, `BEGIN` / `COMMIT` / `ROLLBACK`, `EXPLAIN` / `DESCRIBE`.
- **FR-004**: `CREATE USER` / `ALTER USER` / `DROP USER` MUST be user-identity commands. They MUST NOT be parsed as table DDL.
- **FR-005**: User, shared, and stream tables MUST be created with `CREATE TABLE ... WITH (TYPE = 'USER'|'SHARED'|'STREAM')` (plus existing table options). `CREATE USER TABLE` MUST be rejected with a migration error.
- **FR-006**: `CREATE SHARED TABLE` and `CREATE STREAM TABLE` MAY remain aliases of the `WITH (TYPE = ...)` form in this feature.
- **FR-007**: `CREATE SCHEMA` is the canonical namespace-creation statement. `CREATE NAMESPACE` MUST remain an alias that creates the same object.
- **FR-008**: `SET search_path` / `RESET search_path` are canonical session-schema statements. `USE`, `USE NAMESPACE`, and `SET NAMESPACE` MUST remain aliases that set the same default schema.
- **FR-009**: `CREATE PROCEDURE` / `DROP PROCEDURE` MUST follow PostgreSQL-style procedure syntax used by Kalam functions (`LANGUAGE`, `AS $$ ... $$`, `SECURITY INVOKER|DEFINER`), not a second T-SQL procedure grammar.
- **FR-010**: Topic `CREATE TRIGGER ... ON TOPIC ... EXECUTE PROCEDURE` MUST remain a Kalam extension. It MUST NOT be parsed as a PostgreSQL table trigger.
- **FR-011**: Kalam-only commands (storage, cluster, subscribe, topic, schedule, backup/restore, export, kill job / kill live query) MUST parse with PostgreSQL lexer rules (comments, quoted identifiers, string literals) through the shared parse path.
- **FR-012**: DuckDB-only public syntax, including lambda expressions of the form `x -> expr` used with `array_transform` / `array_filter` / `array_any_match`, MUST be rejected on all entry points.
- **FR-013**: PostgreSQL `UNNEST` in catalog/GUI queries that already work MUST continue to work after the second dialect is removed, either natively or via an internal rewrite that is not a second public language.
- **FR-014**: A statement MUST be parsed at most once for classification plus execution of that statement (visitors/rewrites may transform the already-parsed tree).
- **FR-015**: Parse errors for standard SQL MUST surface as SQL syntax errors. Kalam extension errors MUST name the extension command. The server MUST NOT try every extension parser and then fail with a combined leftover message.
- **FR-016**: Authorization checks that already exist (admin for storage/namespace/cluster, policy roles, etc.) MUST still run, using the parsed statement kind, not a second independent word list as the source of truth.
- **FR-017**: Existing tests and docs that demonstrate removed syntax (`CREATE USER TABLE`, DuckDB lambdas) MUST be updated in this feature. Canonical examples MUST use PostgreSQL syntax.
- **FR-018**: The contributor guide for adding SQL statements MUST describe the single PostgreSQL parse path and MUST NOT instruct engineers to add `starts_with` / whitespace token parsers for standard SQL.
- **FR-019**: Default `CREATE TABLE` kind when `TYPE` is omitted MUST stay the current product default (shared). This feature MUST NOT silently change table-kind defaults.
- **FR-020**: Wire-protocol and HTTP SQL MUST remain able to run the PostgreSQL statements in User Story 1 after this change without a client-side dialect switch.

### Key Entities

- **SQL statement text**: The original command submitted on any entry point.
- **PostgreSQL parse tree**: The single structured representation of standard SQL (and of Kalam extensions that the dialect hook produces).
- **Kalam statement kind**: The product-level classified command (create table, create user, flush storage, subscribe, …) derived from the parse tree.
- **Schema/namespace**: PostgreSQL schema; Kalam namespace. Same object; two names.
- **Table kind**: User, shared, or stream. A table option, not a `CREATE USER` prefix.
- **Kalam extension command**: A documented non-PostgreSQL statement recognized only after PostgreSQL standard parsing does not claim it, or via an explicit dialect hook that cannot collide with a standard prefix.
- **Compatibility alias**: An old Kalam spelling that maps onto a canonical PostgreSQL statement (`CREATE NAMESPACE` → `CREATE SCHEMA`).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A PostgreSQL-compatible client can complete this script without Kalam-only DDL prefixes: `CREATE SCHEMA`, `CREATE TABLE` with columns, `INSERT`, `SELECT`, `CREATE POLICY`, `COMMENT ON`, `SET search_path`, `BEGIN`/`COMMIT`.
- **SC-002**: `CREATE USER <name> WITH PASSWORD '<pw>'` creates a user in 100% of trials and never creates a table.
- **SC-003**: 100% of in-repo examples that previously used `CREATE USER TABLE` are rewritten to `CREATE TABLE ... WITH (TYPE = 'USER')` or an equivalent non-colliding form.
- **SC-004**: Submitting `array_transform([1,2,3], x -> x * 10)` fails on HTTP SQL and on wire with a clear unsupported-syntax error in 100% of trials.
- **SC-005**: GUI-style catalog probes that pass on the current server before this feature still pass after it (same statements, same visible rows or an explicitly documented replacement query).
- **SC-006**: Dialect crate tests for `CALL`, `CREATE SCHEMA`, `GRANT EXECUTE`, `COMMENT ON`, `CREATE POLICY`, `SET search_path`, and `CREATE TABLE` run as one suite and pass without statement-specific string lexers for those commands.
- **SC-007**: Adding a new standard-SQL handler does not require a new `trim().to_uppercase().starts_with` parser; the contributor guide matches the implemented path.
- **SC-008**: No user-visible configuration or session setting selects a second SQL dialect for planning versus classification.
- **SC-009**: Parse+classify of a simple `SELECT` does not regress more than 10% versus the current hot path that skips full DDL parsing (measure in the dialect/core test harness; record times in seconds).

## Assumptions

- PostgreSQL wire (033) and schema-first functions (034) remain; this feature changes the SQL language layer they share, not session/transaction architecture.
- `CREATE TABLE ... WITH (TYPE = 'USER'|'SHARED'|'STREAM')` already exists and is the canonical table-kind syntax.
- Default table kind for bare `CREATE TABLE` stays **shared**, matching current parser behavior.
- `CREATE NAMESPACE` and `USE NAMESPACE` stay as aliases in this feature; they are not deleted yet.
- `CREATE SHARED TABLE` / `CREATE STREAM TABLE` stay as aliases; `CREATE USER TABLE` is removed because it collides with PostgreSQL `CREATE USER`.
- Kalam extension keywords (`STORAGE`, `CLUSTER`, `SUBSCRIBE`, `TOPIC`, `SCHEDULE`) are not renamed to PostgreSQL names in this feature.
- DuckDB-style SQL lambdas were a DataFusion-planning convenience, not a documented Kalam product dialect. Removing them is an accepted breaking change.
- `pg_catalog` shims stay; this is not a catalog-rewrite feature.
- Functions `CREATE PROCEDURE ... AS $$ $$` stays the 034 contract; only the parser implementation changes to PostgreSQL quoting/procedure shape.
- sqlparser 0.62 remains the parser library; Kalam does not fork it to add `Statement::CreateStorage`.
- Contributor docs in `docs/development/how-to-add-sql-statement.md` and `docs/reference/sql.md` are updated in this feature. Out-of-repo skill mirrors are updated when those trees are reachable.

## Scope Boundaries

### In scope

- One PostgreSQL public SQL language on all SQL entry points.
- Remove homemade parsers for statements PostgreSQL already defines (see FR-003).
- Dialect-level handling of Kalam extensions and of PostgreSQL statements that the upstream parser models with the wrong grammar (`CREATE USER`, `CREATE PROCEDURE`).
- Reject `CREATE USER TABLE` and DuckDB-only lambdas.
- Update tests, SQL reference, and the “add a SQL statement” guide.

### Out of scope

- Implementing the full PostgreSQL language (PL/pgSQL, foreign data wrappers, partitioned tables as in PostgreSQL, replication slots, etc.).
- Renaming Kalam operations to PostgreSQL names (`STORAGE FLUSH` → `VACUUM`/`CHECKPOINT`, `SUBSCRIBE TO` → `LISTEN`).
- Changing default `CREATE TABLE` kind from shared to user.
- Removing `pg_catalog` shims or GUI compatibility rewrites that are still required after the dialect is PostgreSQL.
- New product commands.
- Forking or upgrading sqlparser except as needed for PostgreSQL parsing bugs discovered during implementation.

## Dependencies

- 033 unified backend / PostgreSQL wire (entry points and client catalog).
- 034 schema-first functions (`CREATE PROCEDURE` / `CALL` / `GRANT EXECUTE` contracts).
- 024 topic pub/sub and trigger-on-topic syntax.
- Existing `CREATE TABLE ... WITH (TYPE = ...)` options.
