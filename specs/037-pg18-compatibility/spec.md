# Feature Specification: PostgreSQL 18 Compatibility for KalamDB 0.8

**Feature Branch**: `feat/pg-dialect`

**Created**: 2026-10-11

**Status**: Draft

**Input**: User description: "Read docs/pages and specify the KalamDB 0.8 PostgreSQL compatibility program exactly as described there, including the linked GitHub issues, on branch feat/pg-dialect."

**Source of truth**: [KalamDB 0.8 — Epics](../../docs/pages/kalamdb-0.8-epics.md), checked with open trackers #392–#400. This specification is the feature contract for that roadmap. It does not replace the roadmap page. When scope, priorities, dependencies, or acceptance change, the roadmap, the affected trackers, and this specification move together.

**Compatibility target**: PostgreSQL 18. Full compatibility is the goal. Grammar acceptance, a closed issue, or an empty catalog probe is not a compatibility claim.

## User Scenarios & Testing *(mandatory)*

Stories follow the roadmap delivery order. A later story can be demonstrated on its own once its named start gate has passed. Priorities match the roadmap: P1 is foundation and continuous verification, P2 is compatible behavior, and P3 is release readiness. P3 is later delivery, not optional scope. Dependencies apply to the named gates, not to waiting until an entire earlier epic is closed.

### User Story 1 - Submit PostgreSQL SQL once (Priority: P1)

A developer submits PostgreSQL SQL through HTTP SQL or a PostgreSQL connection. KalamDB accepts that SQL through its own dialect, which is built on the PostgreSQL dialect and adds only documented Kalam extensions. It interprets each submitted statement from a single parse, then plans, checks, and executes that same statement. The developer does not rewrite syntax for KalamDB, and KalamDB does not rewrite the statement back into SQL and parse it again.

**Why this priority**: This is gate G1 and tracker #392. Schemas, types, execution, catalogs, and clients all depend on one shared statement. It does not wait for type or schema completion. The existing dialect specification at `specs/036-postgresql-sql-dialect` is reconciled with the current product and with this program; it is not replaced by a second dialect.

**Independent Test**: Run a query and a parameterized data change, including prepared descriptions and a retry, through both HTTP SQL and a PostgreSQL connection. Confirm one parse, one plan for parameters and results, working execution, and rejection of malformed SQL with a located error. Attach that evidence before broad schema or type migration.

**Acceptance Scenarios**:

1. **Given** any production SQL entry point, **When** a developer submits a supported PostgreSQL statement, **Then** KalamDB's own PostgreSQL-based dialect parses it, and validation, authorization, planning, and execution all use that one parsed statement.
2. **Given** a submitted statement, including a KalamDB command, nested SQL, or a retry, **When** KalamDB handles it, **Then** the statement is parsed once and is not turned back into SQL for another parse.
3. **Given** a prepared statement, **When** the client asks for parameter metadata and result metadata, **Then** both descriptions come from the same plan.
4. **Given** a query and a parameterized insert, update, or delete, **When** they run through HTTP SQL and through a PostgreSQL connection, **Then** both paths succeed before the remaining callers are migrated.
5. **Given** identifiers, quoting, literals, casts, parameters, data definition, data changes, joins, subqueries, common table expressions, window queries, conflict handling, or returning clauses that PostgreSQL 18 accepts, **When** the compatibility matrix marks the form supported, **Then** the statement reaches working execution.
6. **Given** an unsupported or malformed form, **When** it is submitted, **Then** KalamDB rejects it explicitly and the error location matches the PostgreSQL expectation for that case.
7. **Given** retired non-PostgreSQL syntax, **When** it is submitted on a production path, **Then** KalamDB rejects it. A second or legacy dialect is not used to accept it.
8. **Given** a `CREATE TABLE` with duplicate column names that differ only by case, **When** it is submitted, **Then** the existing rejection still holds. That regression does not by itself prove the dialect work is complete.

---

### User Story 2 - Use PostgreSQL schemas (Priority: P1)

An ordinary PostgreSQL developer connects with a standard driver and uses schemas, search paths, ordinary tables, types, functions, parameters, data changes, transactions, and introspection without learning Kalam-specific schema or table commands and without a Kalam SDK.

**Why this priority**: This is gate G2 and tracker #400. Production migration starts after G1. The identity and stored-format audit may start during story 1. Types and catalogs need this contract before their broad migration.

**Independent Test**: On KalamDB and on PostgreSQL 18, run the same unchanged fixture: connect, `CREATE SCHEMA`, set `search_path`, create an enum, a composite, and an ordinary table, create and call a typed function, bind parameters, perform data changes inside a transaction, inspect catalogs, then alter, move, and drop objects. Repeat lookup after restart.

**Acceptance Scenarios**:

1. **Given** an authenticated standard-driver session, **When** the developer runs `CREATE SCHEMA`, including `AUTHORIZATION` and `IF NOT EXISTS` where PostgreSQL 18 allows them, **Then** the schema exists and can be resolved by name and by search path.
2. **Given** that schema, **When** the developer runs ordinary `CREATE TABLE` and then looks the table up, **Then** the table exists without `CREATE USER TABLE`, `CREATE SHARED TABLE`, `CREATE NAMESPACE`, or a Kalam SDK.
3. **Given** a restarted server, **When** the developer looks up the same schema and table, **Then** identity, ownership, and visibility match the pre-restart state.
4. **Given** `ALTER SCHEMA`, `DROP SCHEMA` with `RESTRICT` or `CASCADE`, schema privileges, and `SET SCHEMA`, **When** they run against PostgreSQL 18 and KalamDB, **Then** results, error codes, dependencies, and catalog visibility match for the agreed cases.
5. **Given** `search_path`, including the default `"$user", public` path, `current_schema`, `current_schemas`, `pg_catalog`, and temporary-schema lookup, **When** the developer resolves tables, types, enums, and routines, **Then** each object class follows PostgreSQL 18 lookup rules on every SQL entry point.
6. **Given** a connection database name, **When** the session resolves objects, **Then** that name is not silently treated as a schema.
7. **Given** a schema rename or object move, **When** dependent objects are used after restart, **Then** stable object identity remains valid and caches and catalogs do not keep a second copy of the old identity.
8. **Given** PostgreSQL catalog names such as `pg_namespace` and its namespace-named columns, **When** clients query them, **Then** those PostgreSQL names are unchanged.
9. **Given** public interfaces, command-line input and output, SDKs, the administration UI, diagnostics, templates, and examples, **When** a developer reads them, **Then** they say schema where they mean a PostgreSQL schema. A reviewed exception list covers historical migrations, negative tests, and genuine PostgreSQL names.
10. **Given** the large existing code and test corpus that still says namespace, **When** that terminology is replaced, **Then** a reviewed rewrite program applies the change in bounded batches. PostgreSQL's own `pg_namespace` names and the exception list are left unchanged. The old namespace implementation is removed in the same change, not left beside the new schema names.

---

### User Story 3 - Keep PostgreSQL type and routine meaning (Priority: P1)

A developer declares PostgreSQL types and routines and sees the same meaning in tables, function signatures, stored values, query results, and client descriptions. Numeric precision, time zones, JSON, enums, and composite arrays are not silently truncated or reclassified.

**Why this priority**: This is gate G3 and tracker #393. Broad migration starts after G1 and G2. Design experiments may start earlier and must not create a second permanent type model. Execution, catalogs, and client encoding consume this contract.

**Independent Test**: Before broad migration, approve the type contract and prove `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array through planning, active storage, durable flushed storage, prepared descriptions, and text and binary client results, compared with PostgreSQL 18.

**Acceptance Scenarios**:

1. **Given** a column, composite field, parameter, routine argument, or result, **When** its type is resolved, **Then** one logical type model supplies identity and modifiers, and storage layouts and client encodings are derived from that model.
2. **Given** the completed design, **When** a reviewer looks for a second type model or a parallel mapping table, **Then** none remains as a source of type meaning. Temporary adapters have one owner and a removal milestone.
3. **Given** `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and an array of a composite, **When** values are planned, stored, prepared, and returned as text and binary, **Then** precision, order, nulls, casts, and client type identifiers match the agreed PostgreSQL 18 cases, with no silent truncation.
4. **Given** a type family that the matrix marks supported, **When** it is used in every PostgreSQL-legal declaration context, **Then** the same resolver accepts it, and PostgreSQL-invalid contexts are rejected.
5. **Given** `CREATE FUNCTION` with a parenthesized type modifier, **When** the function is created, **Then** KalamDB discards that modifier as PostgreSQL 18 does, instead of enforcing column modifier rules on the signature.
6. **Given** `CREATE FUNCTION` and `CREATE PROCEDURE`, **When** a developer declares arguments, defaults, output arguments, overloads, and return forms, **Then** functions and procedures stay distinct. An existing procedure written with a function-style return contract is migrated explicitly.
7. **Given** `LANGUAGE sql` or `LANGUAGE plpgsql`, **When** the routine is called, **Then** execution evidence matches PostgreSQL 18 for the supported cases. JavaScript and TypeScript remain documented Kalam extensions. An unsupported language is rejected, not accepted and ignored.
8. **Given** a rename, schema move, replace, `RESTRICT`, or `CASCADE`, **When** types or routines change inside a transaction that later rolls back or the server restarts, **Then** identities, dependencies, and permissions stay consistent.

---

### User Story 4 - Match results, errors, and transactions (Priority: P2)

A developer runs compatible SQL and gets PostgreSQL-equivalent results, error codes, permissions, and committed or rolled-back state, including concurrent writes and a transaction that fails partway through.

**Why this priority**: This is gate G4 and tracker #394. It starts after G1, G2, and G3, and it may proceed type family by type family. Catalog and protocol work may overlap earlier, but transaction-sensitive completion uses this evidence.

**Independent Test**: Run the agreed ordinary application fixture against PostgreSQL 18 and KalamDB and compare results, types, error codes, permissions, and committed or rolled-back state. Then run the concurrency and failed-transaction cases required for full closure.

**Acceptance Scenarios**:

1. **Given** the agreed ordinary application slice, **When** it runs on both servers, **Then** results, types, error codes, permissions, and final committed or rolled-back state match.
2. **Given** functions, operators, aggregates, collation, constraints, defaults, identity columns, sequences, and object dependencies that the matrix requires, **When** they are exercised, **Then** behavior matches PostgreSQL 18.
3. **Given** a transaction with rollback, a savepoint, a chosen isolation level, locking, or a statement that fails, **When** the client continues or rolls back, **Then** visible state matches PostgreSQL 18 for that case.
4. **Given** two concurrent writers, **When** they commit or abort, **Then** the surviving rows and errors match the baseline.
5. **Given** views, triggers, routines, roles, privileges, and row security, **When** a behavior is required for the full-compatibility goal and is not yet done, **Then** it is inventoried in the matrix rather than implied by a passing subset.
6. **Given** an open subscription and user-scoped data, **When** a schema or type change commits or rolls back, **Then** the existing websocket does not have to stay connected. The client disconnects and connects again, and the new subscription and user-scoped reads use the schema that committed.
7. **Given** existing row-version ordering and the rule that each shared table has one persisted owner, **When** SQL compatibility work lands, **Then** those invariants stay intact. The cross-group transaction boundary is recorded and is not weakened to claim a match.

---

### User Story 5 - See truthful catalogs (Priority: P2)

A developer or tool inspects `pg_catalog` and `information_schema` and sees the schemas, tables, types, enums, routines, constraints, and dependencies that actually exist, with PostgreSQL 18 column types, object identifiers, and privilege visibility.

**Why this priority**: This is gate G5 and tracker #399. It starts after G1, G2, and G3. It uses story 4 for transaction and lifecycle tests and does not wait for every client-protocol feature. Tracker #396 consumes this work; it does not reimplement catalogs.

**Independent Test**: Create schema, table, constraint, index, enum, composite, array, domain, function, and procedure fixtures. Compare catalog rows with PostgreSQL 18 through both SQL entry points, then rename, drop, roll back, and restart.

**Acceptance Scenarios**:

1. **Given** an implemented object, **When** a client queries the relevant catalog or information-schema view, **Then** the row is present, consistent with related catalogs, and visible only to callers who may see it.
2. **Given** a collection that is actually empty for that caller, **When** the client queries it, **Then** the result is empty. An empty startup probe is not used where real rows are required.
3. **Given** built-in types, **When** clients read type identifiers, **Then** they match PostgreSQL. Custom objects have persistent identifiers that do not collide, and a hash is not treated as proof of uniqueness.
4. **Given** create, alter, rename, set schema, drop, rollback, and restart, **When** catalogs are queried afterward, **Then** references are valid and metadata is not stale.
5. **Given** `pg_namespace` and namespace-named PostgreSQL columns, **When** clients select them, **Then** those names remain the PostgreSQL names.
6. **Given** `psql` describe commands and a pinned driver, ORM, or GUI, **When** they introspect the same fixture used on PostgreSQL 18, **Then** discovered schemas, types, enums, and routines match. A catalog row for a language or routine does not by itself prove that language runs.
7. **Given** credentials or restricted authentication data, **When** a normal client reads catalogs, **Then** those secrets are not exposed.

---

### User Story 6 - Connect with PostgreSQL clients (Priority: P2)

An application connects with PostgreSQL drivers and connection settings, prepares statements, binds parameters, reads results, manages transactions, cancels work, and reconnects.

**Why this priority**: This is tracker #395. It starts after G1 and G3, and it consumes schema and catalog interfaces as they land. It does not wait for every catalog object. Final acceptance includes G5 and the relevant G4 behavior. The earlier wire-protocol delivery is a foundation that excluded full compatibility.

**Independent Test**: Using pinned `psql` and the selected Node.js, Python, Rust, and JDBC drivers, pass connection, query, prepared statement, transaction, cancellation, bulk transfer, and reconnect tests. Confirm parameter and result descriptions use the same type descriptors as execution.

**Acceptance Scenarios**:

1. **Given** valid connection settings, **When** a client starts a session, authenticates, sets session parameters, and disconnects, **Then** each step succeeds or fails with the PostgreSQL error behavior recorded for that case, including encrypted connections where the matrix requires them.
2. **Given** a simple query and an extended-query prepared statement, **When** the client binds parameters and reads rows, **Then** descriptions, encodings, command tags, and error codes match the shared type model.
3. **Given** an unknown type, **When** metadata is sent to the client, **Then** KalamDB does not silently describe it as text.
4. **Given** a cancellation request or a dropped connection, **When** the client reconnects and continues, **Then** session and transaction state match the agreed cases.
5. **Given** `COPY` or another protocol capability listed in the matrix, **When** the client uses it, **Then** the recorded behavior passes. Capabilities not in the supported scope are explicit gaps.
6. **Given** catalog providers from story 5, **When** the same client inspects types and routines, **Then** prepared metadata agrees with `pg_type` and `pg_proc`.

---

### User Story 7 - Migrate and operate with familiar tools (Priority: P2)

A developer inspects a schema, generates and runs a migration, and moves an application from PostgreSQL to KalamDB with familiar tools. Migrated schema and data are checked against PostgreSQL 18.

**Why this priority**: This is tracker #396. It starts after the ordinary-workflow portion of story 4, story 5, and the required client slice of story 6.

**Independent Test**: Point pinned `psql`, a driver, an ORM, and a GUI at the shared catalogs. Migrate schema, data, sequences, constraints, and indexes for a representative application, then run that application and compare the result with PostgreSQL 18.

**Acceptance Scenarios**:

1. **Given** a real schema with custom types and routines, **When** a pinned tool introspects it, **Then** it sees the same objects the catalog gate exposed, not an empty probe.
2. **Given** a PostgreSQL 18 database covered by the migration path, **When** schema, data, sequences, constraints, and indexes are moved, **Then** validation against the reference server passes for that fixture.
3. **Given** an extension, procedural language, backup tool, or replication feature, **When** it is outside the verified path, **Then** the matrix lists it separately instead of hiding it inside a broad compatibility claim.
4. **Given** the finished tooling slice, **When** a remaining ecosystem gap exists, **Then** it is visible in the matrix with an owner.

---

### User Story 8 - Prove compatibility without losing KalamDB behavior (Priority: P1)

A maintainer can see, from the first behavior change onward, what matches PostgreSQL 18, what does not, and whether existing KalamDB behavior still holds.

**Why this priority**: This is tracker #397. The harness and baselines start immediately with story 1. Final sign-off is stage 7, after the functional gates have evidence. P1 here means continuous priority, not that verification must finish before story 1 starts.

**Independent Test**: Before the first behavior change, pin PostgreSQL 18 and client versions, publish the matrix, and record current failures and performance. For a later change, show the new differential test, the migrated existing test, the revision, and any skip reason. At final sign-off, show the full regression run, deletion review, budgets, and the unchanged developer journey.

**Acceptance Scenarios**:

1. **Given** a behavior change, **When** it is accepted, **Then** it brings PostgreSQL 18 differential coverage and migrated existing tests, and the evidence names the revision and client versions.
2. **Given** malformed SQL, hostile input, parameter binding, authorization, and two isolated users, **When** the verification suite runs, **Then** those cases pass or are recorded as explicit gaps.
3. **Given** subscriptions, SDKs, the command-line client, and the administration UI, **When** schemas or types change, **Then** their existing behavior still holds.
4. **Given** an existing test that would fail only because expected SQL text changed, **When** it is migrated, **Then** its behavioral assertion remains, and the suite is not skipped or weakened to turn green.
5. **Given** the release candidate, **When** final verification runs, **Then** required compatibility and regression suites pass, performance and memory budgets pass, obsolete compatibility code is removed or has a dated removal milestone, and no release-blocking correctness, security, or data-loss defect remains.
6. **Given** a change that replaces existing behavior, **When** it is accepted, **Then** the code it replaces is gone, no duplicate path remains, and the result is a simpler implementation of that behavior rather than a new layer added beside the old one.

---

### User Story 9 - Install, upgrade, and recover a release (Priority: P3)

A new developer installs the release, starts a project, creates a schema, runs an application, and uses queries, subscriptions, and the administration UI. An existing installation upgrades, restarts, restores a backup, and recovers from an interrupted operation.

**Why this priority**: This is stage 8 and tracker #398. It starts after final verification, the required acceptance evidence for #392–#396, #399, and #400, and resolution of release blockers. Later order does not make it optional.

**Independent Test**: Using the actual release artifacts, complete a clean disposable-project journey and an upgrade, backup, restore, restart, and interrupted-operation rehearsal. Confirm the published compatibility statement matches the matrix.

**Acceptance Scenarios**:

1. **Given** a clean machine and the release artifacts, **When** a developer installs, initializes, starts everyday development, creates a schema, starts an application, runs queries, uses subscriptions, and opens the administration UI, **Then** that journey completes.
2. **Given** an existing installation, **When** the operator upgrades, restarts, restores a backup, or recovers from an interrupted operation, **Then** data and supported behavior remain usable.
3. **Given** published compatibility docs, migration guidance, examples, release notes, and breaking-change notes, **When** a reader compares them with the matrix, **Then** they describe verified behavior and name any explicit exclusion.
4. **Given** a required gap with no product decision, **When** release sign-off is requested, **Then** the release is not described as fully compatible.

---

### Edge Cases

- Unquoted duplicate column names collide after PostgreSQL case folding; quoted names preserve case, so `"a"` and `"A"` can coexist. Preserve duplicate-name regression coverage with PostgreSQL-correct expectations.
- Retired namespace commands and retired non-PostgreSQL syntax return a migration or rejection error. They are not accepted through a legacy dialect.
- `CREATE USER` creates an identity. It is not a table command. `CREATE USER TABLE` stays rejected.
- A connection database name is not treated as a schema. `pg_catalog` and temporary schemas follow PostgreSQL lookup rules.
- Schema rename and object move keep stable object identities. Stored data and mixed-version messages are not corrupted by a terminology rename.
- `DROP SCHEMA ... RESTRICT` and `CASCADE` follow PostgreSQL dependency behavior, including existing dependency cleanup.
- `CREATE FUNCTION` discards parenthesized type modifiers. Domains carry reusable constraints where PostgreSQL requires that.
- Unconstrained or wide `numeric` values that do not fit a fixed-width decimal have an explicit design or a recorded scope decision. Renaming `decimal` to `numeric` is not that decision.
- `json` and `jsonb`, character padding, float precision, time zones, array dimensions and lower bounds, and enum order are compared explicitly with PostgreSQL 18.
- Unknown types are not described to clients as text.
- Empty catalog results mean the collection is empty for that caller. They do not stand in for unimplemented catalogs.
- Catalog column types match PostgreSQL. An approximate text or integer column that merely satisfies a probe is not sufficient.
- Credentials are not visible through catalog views.
- Custom object identifiers persist, stay unique within their domain, and survive restart. Hashing a name is not a uniqueness guarantee.
- Unsupported languages, attributes, type forms, and protocol features are rejected or listed as gaps. They are not accepted and ignored.
- Existing procedure definitions that used a function-style return contract are migrated explicitly.
- Failed transactions, savepoints, concurrent writes, rollback, and restart leave no stale catalog rows or partial writes beyond the PostgreSQL 18 result.
- User isolation and authorization still hold for catalogs, schemas, and SQL execution.
- Subscriptions and user-scoped data keep a defined interaction with schema, type, and transaction changes.
- Row-version ordering and one persisted owner per shared table are not weakened. Cross-group transaction limits are recorded.
- Kalam-only extensions, including file and embedding values and JavaScript or TypeScript routines, stay optional and visibly distinct from PostgreSQL behavior.
- A temporary compatibility adapter cannot become a second schema, type, or catalog authority.
- A bulk namespace-to-schema rewrite must not rename PostgreSQL's own `pg_namespace` identifiers, unrelated language or runtime namespaces, or cases on the reviewed exception list.
- A wide test or terminology update that is edited by hand across the whole corpus in one pass is not an acceptable migration. The rewrite program must be repeatable and limited to one batch at a time.
- Adding a schema name, type model, parser path, or catalog view beside the old one, and leaving the old one in place, fails the cleanup step for that change.
- PostgreSQL physical storage, heap pages, and extension binary interfaces are not implied by SQL compatibility.

## Requirements *(mandatory)*

### Functional Requirements

#### Compatibility contract

- **FR-001**: KalamDB MUST maintain a versioned compatibility matrix covering SQL commands, types, functions, transactions, protocol messages, catalogs, and client workflows.
- **FR-002**: Each matrix item MUST be marked not assessed, missing, partial, or verified, and MUST name an owner and an automated test once assessment starts.
- **FR-003**: The target MUST be full PostgreSQL 18 compatibility for the agreed scope. Recording a gap MUST NOT be treated as making that gap compatible.
- **FR-004**: Any exclusion MUST be an explicit product decision and MUST appear in accurate release wording. If 0.8 ships with gaps, the release MUST describe the exact supported scope and MUST leave full compatibility as an unfinished goal.
- **FR-005**: KalamDB-specific extensions MUST be documented and distinguishable from PostgreSQL behavior, and the ordinary PostgreSQL journey MUST NOT require them.
- **FR-006**: Acceptance evidence MUST identify the tested revision and the PostgreSQL and client versions. A closed issue, an unchecked roadmap line, or a passing parser test alone MUST NOT count as release evidence.
- **FR-007**: Each epic MUST assess current behavior, link existing work, add child tasks only for uncovered gaps, attach automated evidence, and update the matrix and 0.8 documentation.
- **FR-008**: One accountable owner MUST be assigned to each epic after its gap inventory. This specification does not assume an owner or a duration.

#### Delivery order

- **FR-009**: Delivery MUST follow this order: G1 shared PostgreSQL parse and planning (#392); G2 schema contract (#400); G3 one type model and vertical slice (#393); G4 execution and transactions (#394); G5 catalogs (#399) and PostgreSQL client integration (#395), which may overlap once their start gates pass; tooling and migration (#396); final verification (#397); release sign-off (#398).
- **FR-010**: P0, P1, and P2 roadmap items are all required for the agreed 0.8 scope. P2 means later delivery, not optional scope. A gate dependency MUST NOT be interpreted as waiting for an entire earlier epic to close.
- **FR-011**: G1 MUST be able to start immediately and MUST NOT wait for #393 or #400. Verification baselines MUST start at the same time, before behavior changes. Final verification MUST wait for functional evidence.
- **FR-012**: G2 production migration MUST start after G1. The schema identity and stored-format audit MAY start during G1.
- **FR-013**: G3 broad migration MUST start after G1 and G2. Design experiments MAY start earlier and MUST NOT introduce a second permanent type model.
- **FR-014**: G4 MUST start after G1, G2, and G3, and MAY implement against completed type families rather than waiting for every PostgreSQL type.
- **FR-015**: Catalog work MUST start after G1, G2, and G3, MUST use relevant G4 semantics for lifecycle and transaction tests, and MUST NOT wait for protocol completion.
- **FR-016**: Client-protocol work MUST start after G1 and G3, MUST consume schema and catalog interfaces as they land, and MUST NOT wait for every catalog object. Its final acceptance MUST include G5 and the relevant G4 behavior.
- **FR-017**: Tooling and migration MUST start after ordinary-workflow G4, G5, and the required protocol and client slice.
- **FR-018**: Release sign-off MUST start only after the #397 final gate, required acceptance evidence for #392, #393, #394, #395, #396, #399, and #400, function-value and V8 evidence for SC-012 and SC-013, and resolution of release blockers. Function and V8 work is part of this release, not a follow-up.

#### One PostgreSQL grammar (G1, #392)

- **FR-019**: Production SQL entry points and maintained test helpers MUST parse with KalamDB's own dialect, built on the PostgreSQL dialect. That dialect MUST preserve PostgreSQL lexical rules, operator precedence, quoting, and statement parsing, and MUST add only minimal documented Kalam extensions. Production paths MUST NOT call the upstream PostgreSQL dialect or a legacy dialect directly. Alternate grammar cases MUST remain only as explicit rejection tests or isolated comparisons.
- **FR-020**: KalamDB MUST inventory every parser, tokenizer, SQL rewrite, and text-to-plan entry point, and MUST separate production paths from negative tests and genuinely internal SQL.
- **FR-021**: Validation, authorization, planning, and execution MUST share one parsed statement. Each submitted source unit, including KalamDB commands, MUST be parsed once and MUST NOT be turned back into SQL for another parse. Nested ASTs are reused; string routine bodies and dynamically generated SQL are distinct source units with language-appropriate binding lifetimes. Dependency changes may trigger reanalysis/planning of the retained AST.
- **FR-022**: Planning and execution MUST consume that shared statement. User SQL MUST NOT be rewritten as text on a production path, and there MUST be no runtime fallback to a legacy or second dialect.
- **FR-023**: KalamDB grammar additions MUST stay minimal and MUST NOT capture a statement that PostgreSQL already defines.
- **FR-024**: The existing dialect specification `specs/036-postgresql-sql-dialect` MUST be reconciled with the current product and with this program before grammar migration. Reconciliation MUST replace the current type alias of the upstream PostgreSQL dialect with KalamDB's own dialect built on it. Reconciliation MUST NOT create a second public dialect or a second planner.
- **FR-025**: A query and a parameterized data change MUST succeed through HTTP SQL and through a PostgreSQL connection, including metadata and retries, before the remaining callers move to the shared statement.
- **FR-026**: Prepared parameter descriptions and result descriptions MUST come from the same analyzed plan revision. Dependency invalidation may replace that revision; it MUST NOT leave stale bindings, permissions or result descriptors.
- **FR-027**: The grammar inventory MUST include identifiers and quoting, literals, casts, parameters, data definition, data changes, joins, subqueries, common table expressions, window queries, conflict handling, and returning clauses.
- **FR-028**: Statement validation and error locations MUST match PostgreSQL 18 for covered forms. Unsupported forms MUST be rejected explicitly until they are implemented.
- **FR-029**: Every syntax item in the matrix MUST have positive and negative tests. Accepted statements MUST reach working execution, and tests MUST verify the parse-once rule.
- **FR-030**: Tests and fixtures MUST move with each grammar change, and superseded helpers MUST be removed. G1 evidence MUST be attached before broad schema or type migration.
- **FR-031**: The closed duplicate-column-name regression (#208) MUST stay covered using PostgreSQL quoted/unquoted name semantics and MUST NOT be treated as completion of the dialect epic.
- **FR-032**: Remaining type and routine grammar MUST continue with schemas, types, and execution after G1. Parsing MUST resolve declarations through the one type contract and MUST NOT introduce a dialect-owned type model.
- **FR-033**: G1 is complete for its gate when one PostgreSQL parser configuration, one shared parsed statement, and planning from that statement cover HTTP SQL, PostgreSQL connections, data changes, prepared metadata, nested SQL, and retries; no legacy-dialect fallback or user-SQL text rewrite remains; existing behavior stays covered; and parse-count, malformed-input, allocation, and retention checks pass.

#### PostgreSQL schemas (G2, #400)

- **FR-034**: The product namespace identifier MUST be replaced by one schema identifier throughout production behavior, including related models, stores, services, permissions, caches, planning, stored keys, serialization, subscriptions, jobs, configuration, and generated client contracts.
- **FR-035**: KalamDB MUST use one authoritative schema catalog. A permanent namespace alias, duplicate registry, or parallel mutable namespace and schema definition MUST NOT remain.
- **FR-036**: Schema identity MUST stay distinct from the schema's display name. Rename MUST NOT accidentally invalidate stable object identities.
- **FR-037**: KalamDB MUST make an explicit persisted-format and public-interface migration decision before changing stored fields, storage keys, protocol messages, configuration, exported files, or client contracts. A terminology change MUST NOT corrupt stored data or mixed-version messages.
- **FR-038**: Any temporary upgrade adapter MUST live at one boundary and MUST have an owner and a removal milestone.
- **FR-039**: KalamDB MUST implement PostgreSQL 18 `CREATE SCHEMA`, including `AUTHORIZATION` and `IF NOT EXISTS`, and MUST inventory embedded schema-element support.
- **FR-040**: KalamDB MUST implement `ALTER SCHEMA` rename and ownership, `DROP SCHEMA` with `RESTRICT` and `CASCADE`, schema `USAGE` and `CREATE` privileges, and `SET SCHEMA`, including PostgreSQL dependency behavior and existing dependency cleanup on drop.
- **FR-041**: Quoted and unquoted identifiers, case folding, reserved system-schema names, qualified names, and schema-local object-name collisions MUST match PostgreSQL 18.
- **FR-042**: KalamDB MUST provide the `public` schema and PostgreSQL-compatible `search_path` behavior, including the default `"$user", public` path, effective-schema resolution, `current_schema`, `current_schemas`, and `pg_catalog` and temporary-schema lookup. Different object classes MUST follow PostgreSQL lookup rules.
- **FR-043**: The same resolution MUST apply to tables, types, enums, routines, and prepared statements on every SQL entry point. Database selection MUST remain distinct from schema selection.
- **FR-044**: Ordinary `CREATE TABLE` and ordinary data operations MUST work without `CREATE USER TABLE`, `CREATE SHARED TABLE`, `CREATE NAMESPACE`, or a Kalam SDK. Default table behavior MUST be specified. Kalam table-kind extensions MUST stay optional and documented. `CREATE USER` MUST remain identity creation.
- **FR-045**: Schema terminology MUST be used across HTTP contracts, command-line flags and output, SDKs, the administration UI, diagnostics, templates, examples, and generated documentation.
- **FR-046**: PostgreSQL compatibility names MUST stay unchanged, including `pg_namespace`, `nspname`, `nspowner`, `nspacl`, and namespace-named catalog columns such as those on types, routines, and relations. Unrelated language or runtime namespaces MUST NOT be renamed.
- **FR-047**: G2 is complete for its gate when one schema-identifier contract, authoritative lookup and search-path interfaces, ownership rules, and an explicit stored-format and interface migration decision exist, and a create-schema, ordinary create-table, lookup, and restart slice passes before downstream consumers migrate.
- **FR-048**: The ordinary developer journey MUST pass on both PostgreSQL 18 and KalamDB with a standard driver and the same fixture, including `psql` describe commands and a pinned ORM workflow, with no Kalam-specific SQL, namespace API, or special datatype names.
- **FR-049**: Schema tests MUST cover lookup, permissions, rename, move, drop, rollback, restart, dependency cleanup, stored identity, live subscriptions, and the declared upgrade path.
- **FR-050**: Production behavior MUST NOT keep namespace-owned implementation outside a reviewed exception list for historical migrations, negative tests, and genuine PostgreSQL names. The rename itself MUST follow the batched rewrite program and cleanup step in FR-147 through FR-151.

#### One type model (G3, #393)

- **FR-051**: KalamDB MUST have one authoritative PostgreSQL-oriented logical type model for columns, custom-type fields, parameters, routine signatures, and results, with explicit identity and modifiers. Column nullability MUST stay separate from type identity.
- **FR-052**: Storage layouts and PostgreSQL text and binary encodings MUST be derived from that model. They MUST NOT be separately maintained sources of type meaning. Adding another type enumeration beside the current one MUST NOT satisfy this requirement.
- **FR-053**: One registry MUST cover built-ins, enums, composites, arrays, domains, and documented Kalam extensions such as file and embedding values. Stable custom identities, revisions, field slots, and shared layouts MUST be preserved. Duplicate logical definitions and redundant column fields MUST be retired.
- **FR-054**: Before broad type migration, a design review MUST record the contract, owning areas, an acyclic dependency picture, migration order, storage and client compatibility decisions, and a deletion inventory. The core type model MUST NOT depend on request handlers or connection internals. Each model MUST have one home, and shared behavior MUST have one owner.
- **FR-055**: That review MUST compare three options and record which required meaning each can represent: client wire types alone, execution storage types alone, and one PostgreSQL-oriented catalog with derived adapters. The selected model MUST reduce independently maintained definitions.
- **FR-056**: Prepared descriptions, parameter binding, and result encoding MUST share one type resolution. Duplicate mapping tables MUST be removed. An unknown type MUST NOT be silently advertised as text.
- **FR-057**: Built-in PostgreSQL object identifiers MUST stay correct. Custom object identifiers MUST be persistent and collision-safe. A hash MUST NOT be treated as a uniqueness guarantee. Internal type identity MUST NOT be treated as a wire object identifier, and physical storage types MUST NOT be used to infer every logical type.
- **FR-058**: Immutable type layouts MUST be resolved and cached once per applicable schema or plan revision, with bounded retention and correct invalidation. KalamDB MUST NOT do per-row type lookup, per-row schema reconstruction, or unnecessary text conversion to preserve type meaning.
- **FR-059**: Logical type identity MUST survive expressions, casts, joins, unions, aggregates, storage, and result metadata. When an expression changes type, the result type MUST be defined explicitly.
- **FR-060**: Where current execution already matches PostgreSQL, KalamDB MUST reuse it. Proven gaps MUST be isolated in typed coercion or planning behavior. Metadata registration alone MUST NOT be assumed to supply PostgreSQL operators or casts.
- **FR-061**: Before broad migration, KalamDB MUST prove `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array through parsing, planning, active storage, durable flushed storage, prepared descriptions, and text and binary results, including modifiers, array identity and dimensions, enum order, custom identifiers, nulls, casts, schema-revision invalidation, and negative PostgreSQL comparisons.
- **FR-062**: Performance budgets for allocations, retained memory, resolution cost, throughput, and latency MUST be defined before evaluation and MUST cover primitive and nested values, hot reads, flushed reads, writes, and client encoding.
- **FR-063**: KalamDB MUST inventory every PostgreSQL built-in type, alias, modifier, and type category, and MUST implement numeric, character, binary, boolean, date and time, UUID, JSON, array, enum, composite, domain, range, network, geometric, bit string, XML, text search, and remaining catalog types to the extent the matrix requires.
- **FR-064**: KalamDB MUST match precision, scale, bounds, special values, time zones, null behavior, comparisons, implicit conversions, and explicit casts for supported types.
- **FR-065**: KalamDB MUST explicitly validate float precision, numeric precision and scale including unconstrained values, JSON versus JSONB, character length and padding, temporal precision and time zones, array dimensions and lower bounds, enum ordering, and composite and domain behavior.
- **FR-066**: Full `numeric` behavior that exceeds a fixed-width decimal MUST have an explicit compatible design or a recorded release-scope decision. Renaming `decimal` to `numeric` MUST NOT be treated as that decision. Keeping a fast path for values that fit is allowed.
- **FR-067**: Storage changes MUST be made only for proven representation gaps, with an explicit format version and upgrade decision. Existing values MUST NOT be silently reinterpreted, and a SQL spelling change MUST NOT trigger a blanket rewrite.
- **FR-068**: PostgreSQL compatibility MUST NOT require PostgreSQL's physical storage format.
- **FR-069**: Type identity MUST be preserved through storage, query results, SDKs, the administration UI, and PostgreSQL text and binary encodings, including the type metadata and identifiers clients use.
- **FR-070**: A type-by-type suite MUST verify boundary values, invalid inputs, casts, ordering, and round trips through storage and clients without silent truncation or precision loss.
- **FR-071**: Every supported canonical type MUST work in every PostgreSQL-legal context: table columns, composite fields, arrays, enum and domain references, function and procedure arguments, defaults, output arguments, and return values, through one resolver and registry.
- **FR-072**: A coverage matrix MUST cross every supported type family with each legal context. PostgreSQL-invalid contexts MUST be rejection tests, not silent gaps.
- **FR-073**: KalamDB MUST preserve PostgreSQL context rules. In particular, `CREATE FUNCTION` MUST discard parenthesized type modifiers, and tests MUST show that. Domains MUST be used where PostgreSQL requires a constrained reusable type.
- **FR-074**: Type and routine ownership MUST use the schema contract and the same search-path resolution. Rename and moves MUST preserve identity and update caches and catalog references without a second registry.
- **FR-075**: A temporary type compatibility adapter MUST have one owner, a linked removal task, and a removal milestone, and MUST NOT become a second type authority.
- **FR-076**: G3 is complete for its gate when the reviewed contract, owning areas, and acyclic dependencies exist; storage layouts and client descriptors are derived from that contract; the numeric, jsonb, timestamptz, enum, and composite-array slice passes through planning, storage, and client results; and the migration, deletion, and performance budgets are approved.

#### Types, enums, and routines

- **FR-077**: `CREATE`, `ALTER`, and `DROP TYPE` for composites and enums, and `CREATE`, `ALTER`, and `DROP DOMAIN`, MUST align with PostgreSQL 18. Range, base, and shell type forms MUST be inventoried so none are omitted from the contract by silence.
- **FR-078**: Enum label identity, declaration order, comparisons, and label changes MUST match PostgreSQL 18.
- **FR-079**: Custom syntax, including create-type-if-not-exists, creating a type from a table, inline comments, and argument nullability or emptiness constraints, MUST be audited. Forms that PostgreSQL 18 already has MUST be migrated. Any retained Kalam syntax MUST be an explicit documented extension.
- **FR-080**: Functions with return types and procedures invoked with `CALL` and output arguments MUST stay distinct. Existing procedure definitions that used a function-style return contract MUST be migrated explicitly, not silently reinterpreted.
- **FR-081**: Routine coverage MUST include names and search path, argument modes and defaults, positional and named calls, overload identity, set-returning and table returns, replace restrictions, body quoting, and routine attributes, with positive and negative differential tests. Parsing success alone MUST NOT be sufficient.
- **FR-082**: Supported languages MUST be specified and tested independently of parsing. `LANGUAGE sql` and `LANGUAGE plpgsql` MUST have execution evidence for the supported scope. JavaScript and TypeScript MUST remain explicit Kalam extensions. Unsupported languages and attributes MUST NOT be accepted and ignored.
- **FR-083**: Type dependencies, rename, schema moves, overload resolution, replacement, `RESTRICT`, `CASCADE`, permissions, and plan invalidation MUST be verified.

#### Execution and transactions (G4, #394)

- **FR-084**: The agreed ordinary application slice MUST match PostgreSQL 18 results, types, error codes, permissions, and committed or rolled-back state.
- **FR-085**: Full closure of this epic MUST cover the required matrix, including concurrent writes and failed-transaction behavior.
- **FR-086**: Functions, operators, aggregates, collation, and expression evaluation MUST match the baseline for required items.
- **FR-087**: Constraints, defaults, identity columns, sequences, schema resolution, and object dependencies MUST match the baseline for required items.
- **FR-088**: Transaction boundaries, rollback, savepoints, isolation, locking, concurrent writes, and failed-transaction behavior MUST match the baseline for required items.
- **FR-089**: Views, triggers, routines, roles, privileges, and row security MUST be inventoried against the full-compatibility goal, and each required behavior MUST be implemented or recorded as an explicit gap.
- **FR-090**: A schema or type change MUST NOT be required to keep an existing subscription connected. The supported interaction is to disconnect the websocket and connect again. The new subscription and user-scoped reads MUST use the schema that committed, including when the change rolled back. This interaction MUST be tested.
- **FR-091**: Existing row-version ordering and one persisted owner per shared table (#359, #391) MUST keep their invariants and their own implementation ownership. The cross-group transaction boundary MUST be identified and MUST NOT be weakened to claim compatibility.
- **FR-092**: Differential tests MUST match results, errors, and committed state, including concurrency and rollback.
- **FR-093**: G4 is complete for its gate when the ordinary application slice matches PostgreSQL 18. Full epic closure additionally requires the complete required matrix.

#### Catalogs and introspection (G5, #399)

- **FR-094**: KalamDB MUST build a versioned inventory of every PostgreSQL 18 system catalog, system view, information-schema view, and introspection helper, recording columns, types, expected behavior, owner, status, tests, and linked work. A small client-query subset MUST NOT be described as full catalog parity.
- **FR-095**: The inventory MUST prioritize, and then still account for the rest of, schemas and relations (`pg_namespace`, `pg_database`, `pg_class`, `pg_attribute`, `pg_attrdef`, `pg_tables`, `pg_views`); types (`pg_type`, `pg_enum`, `pg_range`, `pg_cast`, `pg_collation`, including built-in, named, array, composite, domain, and table row types); routines (`pg_proc`, `pg_language`, including kind, argument identity and modes, defaults, and result types); relationships (`pg_constraint`, `pg_index`, `pg_indexes`, `pg_depend`, `pg_description`, `pg_sequence`, `pg_sequences`, `pg_trigger`); visibility and session metadata (`pg_roles`, `pg_auth_members`, restricted `pg_authid`, `pg_settings`, and `search_path`); and information-schema coverage for schemata, tables, columns, views, routines, parameters, user-defined types, attributes, domains, element types, constraints and key usage, triggers, sequences, and privilege views.
- **FR-096**: Introspection helpers and casts MUST match PostgreSQL 18 signatures, overloads, output types, and errors, including `format_type`, `pg_get_expr`, `pg_get_constraintdef`, `pg_get_indexdef`, `pg_get_viewdef`, `pg_get_functiondef`, function argument and result helpers, `pg_typeof`, `to_regclass`, `to_regtype`, `to_regprocedure`, `reg` casts, and visibility and privilege helpers.
- **FR-097**: Catalog views MUST be produced from authoritative schema, type, and routine records. There MUST NOT be a second mutable catalog or a connection-only mirror. Both SQL entry points MUST consume the same providers.
- **FR-098**: Every catalog type identifier and modifier MUST come from the canonical type model. Catalog references MUST stay valid across schemas, relations, attributes, enum labels and order, array links, domains, routine arguments and results, constraints, and dependencies.
- **FR-099**: Metadata MUST reflect create, alter, rename, set schema, and drop, plus transaction visibility, rollback, restart, and cache invalidation. `RESTRICT` and `CASCADE` behavior MUST be defined.
- **FR-100**: Catalog column types and semantics MUST match PostgreSQL 18, including arrays, character codes, modifiers, nullability, and defaults. Approximate text or integer columns that only pass a client probe MUST NOT be sufficient.
- **FR-101**: Visibility and privileges MUST match PostgreSQL, with Kalam user isolation applied consistently. Credentials MUST NOT be exposed through `pg_authid` or other metadata.
- **FR-102**: Empty results MUST be valid only when the represented collection is actually empty for that caller. Empty stubs MUST be replaced where implemented features require rows. Unsupported capabilities MUST be tracked and MUST NOT be fabricated as objects, statistics, languages, or extension support.
- **FR-103**: Catalog comparisons with a pinned PostgreSQL 18 server MUST normalize generated object identifiers by object identity while still checking uniqueness and cross-catalog references.
- **FR-104**: Tests MUST cover schema, table, constraint, index, enum, composite, array, domain, function, and procedure fixtures through both SQL entry points and prepared descriptions, plus rename, alter, drop and recreate, rollback, restart, concurrent definition changes, privileges, and search path, with no dangling references or stale metadata.
- **FR-105**: `psql` describe commands and pinned driver, ORM, and GUI introspection MUST be verified against those fixtures. A catalog row MUST NOT be treated as proof that a language runtime works.
- **FR-106**: Catalog queries with many tables, types, overloads, and principals MUST stay within bounded caches and MUST NOT rebuild the full catalog per row when a shared snapshot is sufficient.
- **FR-107**: PostgreSQL storage files, heap pages, and extension binary interfaces MUST be treated as separate compatibility dimensions and described honestly.
- **FR-108**: G5 is complete for its gate when shared providers expose truthful, consistent schema, relation, type, enum, and routine metadata, object identifiers, and helpers, with visibility, definition-change, rollback, and restart tests, and both SQL entry points use those providers.
- **FR-109**: Catalog providers MUST consume the single schema catalog from G2 and MUST keep `pg_namespace`, information-schema schemata, and schema references consistent through create, rename, move, drop, rollback, and restart.

#### PostgreSQL connections and clients (#395)

- **FR-110**: KalamDB MUST support connection startup, authentication, encrypted connections where required by the matrix, session settings, cancellation, and connection termination.
- **FR-111**: KalamDB MUST support simple and extended query flows, prepared statements, parameter binding, portals, result metadata, command tags, and error codes.
- **FR-112**: `COPY` and other protocol capabilities recorded in the matrix MUST behave as specified. Omitted capabilities MUST be explicit gaps.
- **FR-113**: Prepared parameter descriptions, result descriptions, binding, and encoding MUST use the same type descriptors. Built-in and custom object identifiers, modifiers, array and composite metadata, and text and binary round trips MUST match the resolved logical type.
- **FR-114**: Independent client type mappings and silent unknown-type-to-text fallbacks MUST be removed. Protocol conversion MUST stay at the connection boundary.
- **FR-115**: Pinned `psql` and the selected Node.js, Python, Rust, and JDBC drivers MUST pass connection, query, prepared statement, transaction, cancellation, bulk transfer, and reconnect tests.
- **FR-116**: The closed wire-protocol delivery (#354) MUST be treated as a foundation that excluded full compatibility. This epic MUST validate that foundation and close the remaining gaps, not re-deliver the original scope.
- **FR-117**: Hardening of the `kalam.user_id` identity override against row-security impersonation (#375) MUST remain owned by that issue.
- **FR-118**: Final client acceptance MUST include G5 catalog integration and relevant G4 transaction and error-state behavior. Prepared metadata for every canonical built-in and custom type MUST agree with `pg_type` and `pg_proc`.

#### Tooling and migration (#396)

- **FR-119**: Pinned `psql`, driver, ORM, and GUI workflows MUST introspect and migrate real schemas, custom types, and routines. Migrated schema and data MUST be validated against PostgreSQL 18.
- **FR-120**: KalamDB MUST provide a repeatable PostgreSQL-to-KalamDB migration path covering schema, data, sequences, constraints, and indexes.
- **FR-121**: Extension, procedural language, backup, and replication compatibility MUST be inventoried separately so they cannot disappear behind a broad compatibility claim.
- **FR-122**: Catalog implementation MUST remain owned by #399. Tooling MUST consume those catalogs. Empty probe schemas, the closed wire-protocol delivery (#354), and REST result-schema work (#50) MUST NOT be treated as catalog, ORM, or migration evidence.
- **FR-123**: Representative applications MUST migrate and run successfully, and every remaining tooling or ecosystem gap MUST be visible in the matrix.

#### Continuous verification (#397)

- **FR-124**: PostgreSQL 18 and client versions MUST be pinned. The matrix, test inventory, current failures, and performance baseline MUST be recorded before behavior changes.
- **FR-125**: Each implementation change MUST bring PostgreSQL 18 differential coverage and migrated existing tests. Evidence MUST record revision, results, skips and reasons, semantic differences, and benchmark results.
- **FR-126**: A versioned differential suite MUST run in continuous integration and MUST retain reproducible failure cases.
- **FR-127**: Verification MUST cover malformed SQL, hostile inputs, parameter binding, authorization, and isolation between users.
- **FR-128**: Subscriptions, SDKs, the command-line client, and the administration UI MUST be verified across schema and type changes.
- **FR-129**: Query latency, parsing cost, allocations, and retained memory MUST be measured against agreed budgets. Caches MUST stay bounded, and obsolete code MUST be removed or given a removal milestone.
- **FR-130**: All existing test surfaces MUST migrate with the syntax and type changes: server tests, command-line smoke and end-to-end tests, PostgreSQL connection and extension tests, SDKs, administration UI tests, fixtures, snapshots, seed SQL, templates, examples, and benchmarks. Terminology and syntax updates that apply the same way across many of those tests MUST use the batched rewrite program in FR-149, and each batch MUST preserve behavioral assertions.
- **FR-131**: Ordinary SQL tests MUST use the production PostgreSQL grammar. Behavioral coverage MUST be preserved. Every removed or changed assertion MUST be recorded with its replacement. Suites MUST NOT be skipped, and assertions MUST NOT be weakened, solely to obtain a green result.
- **FR-132**: Differential comparison MUST cover accepted syntax, rejected syntax, results, result types, error categories and error codes, casts, overflow, nulls, sorting, and transaction state. Only nondeterminism that does not hide a semantic difference MAY be normalized.
- **FR-133**: Focused regressions MUST cover custom type identity, revisions, field slots, nested values, enums, schema evolution, active storage, durable flushed storage, and text and binary round trips.
- **FR-134**: A regression guard MUST reject new legacy-dialect entry points and obsolete SQL in maintained fixtures, with an explicit reviewed exception list for negative tests and Kalam extensions.
- **FR-135**: Required test runs MUST use a fresh matching server and a rebuilt command-line client, including command-line smoke, SDK end-to-end, administration UI, and the full test sweep. Records MUST include tested revision, enabled capabilities, suite counts, failures, skips with reasons, and runtime for performance runs.
- **FR-136**: Final stage 7 MUST require full regression, architecture and deletion review, performance budgets, and an unchanged ordinary PostgreSQL developer journey on the integrated release candidate. Passing parser tests alone MUST NOT satisfy this epic.
- **FR-137**: Verification MUST confirm one type contract across SQL, catalogs, execution, storage, SDK metadata, and client encoding, plus the type-family matrix, shared positive and negative fixtures, and catalog and prepared-metadata agreement.
- **FR-138**: Authorization-cache bounds (#373), durable live-event acknowledgements (#368), and removal of remaining deep copies on query and data-change paths (#383) MUST keep their own issue ownership. This epic owns the cross-cutting matrix, differential harness, regression evidence, and performance gates.

#### Release readiness (#398)

- **FR-139**: The actual release artifacts MUST pass clean installation and onboarding, upgrade, backup and restore, restart and recovery, and supported-platform checks.
- **FR-140**: A clean project MUST be walked from installation and initialization through everyday development startup, schema creation, application startup, queries, subscriptions, and the administration UI.
- **FR-141**: Restart durability, upgrade compatibility, backup and restore, and recovery from interrupted operations MUST pass on the release candidate.
- **FR-142**: PostgreSQL compatibility documentation, migration guidance, working examples, release notes, and breaking changes MUST match verified behavior.
- **FR-143**: Release artifacts, supported platforms, diagnostics, and operational instructions MUST be verified.
- **FR-144**: The release MUST NOT be published as fully compatible while a required gap lacks an explicit product decision and accurate wording.
- **FR-145**: Row versioning (#359), shared-table ownership (#391), and durable live delivery (#368) MUST keep their implementation scope. Their release impact MUST be assessed during readiness review.
- **FR-146**: Release sign-off MUST include every epic owner, implementation tasks, and attached evidence; an agreed baseline and scope; matrix satisfaction for syntax, types, execution, protocol, and tooling; function-value and V8 evidence for SC-012 and SC-013; explicit exclusions; passing differential and regression tests; no release-blocking correctness, security, or data-loss issue; passing performance and memory budgets; passing fresh-project, upgrade, backup, restore, and restart checks; and ready documentation, examples, and artifacts.

#### Cleanup on every change

- **FR-147**: Every change that adds or replaces behavior MUST include a cleanup step in that same change, before it is accepted. The cleanup step MUST remove superseded paths, duplicate models, unused adapters, dead helpers, and obsolete names introduced or left behind by the change.
- **FR-148**: The preferred result of a change MUST be a smaller, clearer implementation of the existing behavior. A new layer, alias, mapping, or fallback MUST NOT be added beside the code it replaces unless that addition is a temporary adapter with one owner and a removal milestone, as required for schema and type migration.
- **FR-149**: Wide, repetitive edits, including the namespace-to-schema rename and the matching test, fixture, and example updates, MUST be done by a reviewed rewrite program in bounded batches. Each batch MUST be small enough to review and verify on its own. The program MUST be safe to run again, and a later batch MUST NOT depend on holding the rest of the corpus in one editing session.
- **FR-150**: After each batch, a search MUST show that the replaced namespace terminology and duplicate implementations are gone outside the reviewed exception list. Finding a leftover product namespace identifier, a second type or catalog authority, or an unused compatibility path fails that batch.
- **FR-151**: Cleanup MUST look for a better structure, not only a rename. When two paths do the same work, one owner remains. When a helper exists only to bridge a removed name, the helper goes away with the name.

#### Review corrections: correctness and measurable efficiency

- **FR-152**: Historical rows MUST distinguish absent fields from explicit NULL and preserve DDL-time missing values independently of current write defaults. Type changes MUST validate conversion, constraints and dependencies before atomic success; incompatible conversion MUST NOT be postponed to SELECT.
- **FR-153**: Persisted row/type/schema revisions MUST be validated, durable before referenced data and available for supported snapshots/recovery. Revision exhaustion MUST fail without wrap or mutation. New formats MUST have a capability barrier and an explicit downgrade policy.
- **FR-154**: Type representations MUST preserve stable enum member identity, array dimensions/lower bounds and nested revisions. Physical layouts and logical/wire types MUST NOT be conflated.
- **FR-155**: Retained memory MUST be bounded in bytes across caches, schema history in RAM, prepared statements, portals, scan buffers and concurrent active work. Cache eviction alone MUST NOT count as reclaiming pinned memory. Selective results MUST not retain disproportionate source buffers indefinitely.
- **FR-156**: G4 acceptance MUST include the failure/concurrency matrix in [execution-performance.md](contracts/execution-performance.md), including failed transactions, savepoints, isolation, atomic DML/DDL, retry ambiguity and live-event outcomes. A single rollback/concurrent-write smoke test MUST NOT establish compatibility.
- **FR-157**: Performance evidence MUST compare the prior and candidate KalamDB designs on identical work, report per-workload PostgreSQL 18 ratios, and meet preregistered latency/allocation/memory budgets. Syntax changes and codec microbenchmarks alone MUST NOT be described as PostgreSQL performance parity.
- **FR-158**: Migration verification MUST compare typed data contents, constraints, indexes and sequence state in addition to counts. Standard-client acceptance MUST include error recovery, portal lifetimes, backpressure and isolation of session state.
- **FR-159**: G2 MUST prove stable identity and PostgreSQL quoted-name behavior before broad renaming. Ordinary tables MUST default to Shared with normal privileges; per-user storage MUST remain an explicit extension. Historical names/keys MUST migrate without silent merging or data movement.

#### Function values and V8 integration

- **FR-160**: Function arguments, returned values, nested calls and database query objects MUST share G3 logical types and immutable value owners where representations are compatible. Read-only forwarding MUST avoid payload copies and redundant serialization; any conversion MUST preserve logical meaning.
- **FR-161**: The JS/TS boundary MUST support explicit read-only database views with lazy access and safe forwarding to another routine. Existing mutable plain-object APIs MUST retain their semantics unless migrated through a documented ABI revision; materialization into mutable objects MUST be explicit for views.
- **FR-162**: Buffer reuse/transfer MUST enforce ownership, mutability, authorized range and allocator constraints. JavaScript MUST NOT gain writable access to shared database memory. Transfer from guest-owned buffers MUST be explicit and revoke guest access safely or use a bounded snapshot copy.
- **FR-163**: Shared values MUST remain valid through await and permitted nested calls, without allowing stale handles, cross-invocation access or capability leakage. Completion, cancellation, timeout and isolate reuse MUST release or continue to account every retained resource; child permission realms MUST remain intact.
- **FR-164**: Function memory limits MUST cover managed heap, imported/external buffers, pinned query data, conversion scratch, queues and results together. An external-buffer fast path MUST NOT bypass existing admission limits or silently broaden sandbox permissions.
- **FR-165**: Function conversion MUST preserve numeric range/precision, microsecond temporal precision, JSON/SQL null distinctions, array shape/bounds, enum identity and nested types. Invalid conversions MUST fail explicitly instead of truncating, substituting zero/NULL or discarding metadata.
- **FR-166**: Streaming/batch function access MUST be bounded and cancellation-safe, with explicit invocation/volatility/transaction semantics. Copy-byte, CPU, allocation, tail latency and retained-memory evidence MUST cover query→function→nested-call→result and cold/warm execution.
- **FR-167**: Function ABI and generated declarations, named-argument binding, runtime conversion and documentation MUST derive from the same contracts. Superseded scalar reconstruction and redundant serialization MUST be removed after consumer migration; SQL/plpgsql and Kalam JS/TS language support MUST remain distinct.

### Key Entities

- **Compatibility matrix item**: One SQL, type, function, transaction, protocol, catalog, or client-workflow behavior, with status, owner, automated test, and any explicit exclusion.
- **Schema**: A PostgreSQL schema with a stable identity distinct from its display name, an owner, privileges, and a place in `search_path`.
- **Function value view**: A typed argument/result carrying an immutable value owner, authorized range, descriptor revision and invocation lifetime; V8 wrappers are derived views rather than independent value authorities.
- **Logical type**: The single meaning of a PostgreSQL or documented Kalam type, including identity, modifiers, and category. Storage layouts and client encodings are derived views of it.
- **Routine**: A function or procedure with schema ownership, argument modes, defaults, overload identity, return form, language, and dependencies.
- **Catalog projection**: A `pg_catalog` or `information_schema` view produced from authoritative records, with PostgreSQL column types, object identifiers, privileges, and dependency links.
- **Client session**: A PostgreSQL connection or HTTP SQL session, including authentication, database selection, search path, prepared statements, and transaction state.
- **Transaction outcome**: The committed, rolled-back, or failed state visible to the client, including savepoints, isolation, and concurrent writers.
- **Migration package**: Schema, data, sequences, constraints, and indexes moved from PostgreSQL 18 and checked against that reference.
- **Release evidence**: The revision, client versions, suite results, budget comparison, and journey or recovery rehearsal attached to a gate.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An ordinary PostgreSQL developer, using a standard driver and no Kalam-specific syntax or SDK, completes schema creation, search-path setup, an enum, a composite, an ordinary table, a typed function call, parameter binding, data changes, a transaction, and catalog introspection. The same fixture on PostgreSQL 18 matches results, error categories, and committed or rolled-back state.
- **SC-002**: Every matrix item is marked not assessed, missing, partial, or verified. Each verified or partial item has an owner and an automated test. Any 0.8 exclusion is a recorded product decision, and the release text matches that matrix.
- **SC-003**: Each submitted statement is interpreted from one parse on HTTP SQL and PostgreSQL connections, including prepared metadata, nested SQL, and retries. Production paths do not fall back to a second dialect or rewrite user SQL into text for another parse. Malformed SQL is rejected with a located error.
- **SC-004**: `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array round-trip through active storage, durable flushed storage, and text and binary client results with no silent truncation or precision loss, and they match the agreed PostgreSQL 18 comparison and cast cases.
- **SC-005**: The client suite passes connection, query, prepared statement, transaction, cancellation, bulk transfer, and reconnect tests on pinned `psql` and the selected Node.js, Python, Rust, and JDBC drivers.
- **SC-006**: A representative application migrates schema, data, sequences, constraints, and indexes, then runs. Migrated schema and data match the PostgreSQL 18 reference for the agreed fixtures, and every remaining tooling gap is listed in the matrix.
- **SC-007**: Preregistered budgets in [execution-performance.md](contracts/execution-performance.md) pass on the release candidate: targeted materialization paths reduce allocations by at least 20% and CPU/row or median runtime by at least 10%, with the defined regression limits and bounded active/cached memory. Report measured per-workload ratios to both prior KalamDB and PostgreSQL 18; a necessary budget exception requires an explicit design decision before acceptance.
- **SC-008**: Required compatibility suites and existing regression suites pass on the release candidate. No release-blocking correctness, security, or data-loss defect remains.
- **SC-009**: Using the actual release artifacts, a clean install completes the developer journey, and an upgrade completes restart, backup, restore, and recovery from an interrupted operation.
- **SC-010**: Catalog introspection used by `psql` and a pinned ORM returns schema, table, type, enum, and routine metadata consistent with the objects that exist, including after rollback and restart, and does not expose credentials.
- **SC-011**: The namespace-to-schema rename and the large test and fixture updates are applied by a repeatable rewrite program in bounded batches. After the final batch, production code has no product namespace identifiers outside the reviewed exception list, and each accepted change has removed the code it replaced instead of leaving a duplicate beside it.

- **SC-012**: A database row or composite array read under G3 passes through a native function, a JS/TS read view, a nested routine and a returned value with correct types/permissions and no payload-byte copy on representation-compatible forwarding boundaries. Mutation uses explicit materialization or validated exclusive transfer, and stale/forged handles fail.
- **SC-013**: T112/T122 demonstrate the function-path targets in [functions-v8.md](contracts/functions-v8.md), including at least 20% fewer allocated bytes and 10% lower CPU/median runtime for targeted query/call chains, under the existing regression limits and bounded memory through cancellation, delayed GC and isolate reuse. These are acceptance targets, not measured results.

## Assumptions

- PostgreSQL 18 SQL, data types, system catalogs, information schema, and the frontend/backend protocol are the behavioral reference. The roadmap links are the detailed references for schema, type, routine, and catalog behavior.
- `specs/036-postgresql-sql-dialect` remains the dialect specification to reconcile. Its principles of one public PostgreSQL grammar, parse once, and Kalam-only commands as explicit extensions remain in force. This program adds the 0.8 gates that specification does not close by itself. Where 036 still says namespace for a PostgreSQL schema, G2 terminology wins after reconciliation.
- The 10 October 2026 code inspection is a planning snapshot, not a passing test. It found remaining legacy-dialect paths, prepared metadata that can plan the same statement twice, retries that can reprocess statement text, and separate type representations on column definitions and client encodings. Those findings MUST be rechecked at implementation time because the product is still changing.
- Owners, estimates, and child tasks are assigned after each gap inventory. No owner or duration is implied here.
- Related issues keep their own scope: #208 duplicate column names, #50 REST result schema, #354 initial wire protocol, #359 row versions, #391 shared-table ownership, #375 identity override, #373 authorization cache bounds, #368 durable live events, and #383 deep copies. This program consumes or preserves them and does not close them by declaration.
- PostgreSQL physical storage is not required. Storage format changes require a proven representation gap and an explicit upgrade decision.
- External page copies are historical snapshots. `docs/pages/kalamdb-0.8-epics.md` and the GitHub trackers are the living roadmap.
- Issue status above was checked on 11 October 2026. #392, #393, #394, #395, #396, #397, #398, #399, and #400 were open.
- The existing test, fixture, and example corpus is too large to rename by reading and editing every file in one pass. Wide namespace-to-schema and syntax updates are mechanical and MUST go through a reviewed rewrite program in batches. Behavior that is not a mechanical replacement is still reviewed and tested with the batch it belongs to.

## Traceability

| Gate | Roadmap stage | Tracker | Outcome |
| --- | --- | --- | --- |
| G1 | 1, P0 | [#392](https://github.com/kalamdb/KalamDB/issues/392) | One PostgreSQL grammar and one shared parsed statement |
| G2 | 2, P0 | [#400](https://github.com/kalamdb/KalamDB/issues/400) | Schemas replace namespaces for the ordinary developer journey |
| G3 | 3, P0 | [#393](https://github.com/kalamdb/KalamDB/issues/393) | One type model and a proven vertical slice |
| G4 | 4, P1 | [#394](https://github.com/kalamdb/KalamDB/issues/394) | Results, errors, and transaction state match PostgreSQL 18 |
| G5 | 5A, P1 | [#399](https://github.com/kalamdb/KalamDB/issues/399) | Truthful `pg_catalog` and `information_schema` |
| Client | 5B, P1 | [#395](https://github.com/kalamdb/KalamDB/issues/395) | Standard drivers connect, prepare, and transact |
| Tooling | 6, P1 | [#396](https://github.com/kalamdb/KalamDB/issues/396) | Introspection and PostgreSQL-to-KalamDB migration |
| Verify | Continuous, final stage 7, P0 | [#397](https://github.com/kalamdb/KalamDB/issues/397) | Differential tests and KalamDB regressions |
| Release | 8, P2 | [#398](https://github.com/kalamdb/KalamDB/issues/398) | Install, upgrade, backup, restore, and recovery |
