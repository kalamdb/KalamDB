# Tasks: PostgreSQL 18 Compatibility for KalamDB 0.8

**Input**: Design documents from `/specs/037-pg18-compatibility/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md, and the dialect contracts in `specs/036-postgresql-sql-dialect/`

**Tests**: Included. The specification requires differential evidence, parse-once proof, and cleanup checks for every gate. Write the story's tests first and confirm they fail before the implementation task that makes them pass.

**Organization**: Tasks follow the gate order in plan.md. User Story 8 is P1, and its "before the first behavior change" half is Phase 2. Its closure phase is after the behavior it has to prove, because the spec's final gate waits for functional evidence.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: User story label. Setup, Foundational, and Polish tasks have no story label
- Every task names a file path

## Dialect and DataFusion rules

Read this before any parse or plan task. Checked against sqlparser 0.62.0 and DataFusion 55.2.0 sources.

sqlparser does not inherit a `Dialect` impl through a containing struct. A hand-written `impl Dialect for KalamDbDialect` that forwards only some methods uses the trait defaults for the rest, and those defaults are not PostgreSQL. PostgreSQL syntax is selected with `dialect.is::<PostgreSqlDialect>()` (`dialect_of!` in the tokenizer and parser): casts, arrays, dollar-quoted strings, nested comments, JSON operators, and identifier quoting. The supported way to keep that identity is the `derive-dialect` feature:

```rust
use sqlparser::derive_dialect;
use sqlparser::dialect::PostgreSqlDialect;

derive_dialect!(KalamDbDialect, PostgreSqlDialect, preserve_type_id = true);
```

`preserve_type_id = true` makes `KalamDbDialect` report PostgreSQL's `TypeId` from `Dialect::dialect()`. Without it, the parser treats the dialect as a new dialect and PostgreSQL syntax breaks. `derive_dialect!` overrides are boolean capability flags and `identifier_quote_style` only. Do not override quoting or precedence. Do not add a boolean override unless `specs/036-postgresql-sql-dialect/contracts/sql-language.md` documents that extension.

`Dialect::parse_statement` returns `Option<Result<Statement, ParserError>>` and can only yield a `sqlparser::ast::Statement`. It cannot carry a Kalam payload. Return `None` for every statement PostgreSQL already defines. Only explicit Kalam-only forms are recognized by `KalamParser` on the same token cursor. Ordinary `CREATE TYPE`, `CREATE FUNCTION`, `CREATE PROCEDURE` and `CALL` retain PostgreSQL grammar regardless of implementation language; `LANGUAGE javascript` does not authorize a second procedure parser. Do not invent a dummy `Statement` variant, reparse the text, or stash a payload on the dialect. T125 adds production-path coverage beyond the completed G1 corpus.

DataFusion 55.2 plans an AST. `SessionState::statement_to_plan` takes `datafusion_sql::parser::Statement`. Standard SQL is `Statement::Statement(Box<sqlparser::ast::Statement>)`. `DFParserBuilder::new` defaults to `GenericDialect`. `SessionState::sql_to_expr` resolves a dialect name with `dialect_from_str`, which only knows built-in names such as `"postgresql"` and cannot see `KalamDbDialect`. `datafusion.sql_parser.dialect = postgresql` in `backend/crates/kalamdb-core/src/sql/datafusion_session.rs` stays a defensive default so an accidental internal parse is PostgreSQL rather than generic. User SQL does not enter through `session.sql`, `DFParser::parse_sql`, `create_logical_plan` on SQL text, or `dialect_from_str`. If a DataFusion parser must be used, call `DFParserBuilder::with_dialect(&KalamDbDialect::new())`. Retries and nested AST nodes reuse the parsed source. Prepared parameter and result metadata share one analyzed plan revision; invalidation can reanalyze/replan from the retained AST. Nested string bodies and dynamic SQL follow the source-unit rules in `contracts/execution-performance.md`.

## Path Conventions

- Parser and classification: `backend/crates/kalamdb-dialect/`
- Schema and type models: `backend/crates/kalamdb-commons/`
- Planning and execution: `backend/crates/kalamdb-core/`
- Catalogs: `backend/crates/kalamdb-system/`, `backend/crates/kalamdb-views/`
- Wire: `backend/crates/kalamdb-postgres-wire/`
- Batched rename tool: `scripts/pg18-rewrite/rewrite.sh`
- Feature docs: `specs/037-pg18-compatibility/`

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Enable the sqlparser dialect derive path and the rewrite tool. No new runtime crate.

- [X] T001 Add the sqlparser `derive-dialect` feature next to `visitor` in `backend/crates/kalamdb-dialect/Cargo.toml`
- [X] T002 Verify resolved `Cargo.lock` versions and the dialect derive/AST/Arrow view APIs match the `Cargo.toml` dependency constraints (DataFusion 55.2.0, sqlparser >=0.62.0, arrow-schema >=59.2.0), and note any drift in `specs/037-pg18-compatibility/research.md`
- [X] T003 Create the idempotent one-batch rewriter `scripts/pg18-rewrite/rewrite.sh` from `specs/037-pg18-compatibility/contracts/cleanup-and-rewrite.md`, including dry-run, exact-token edits, expected-match counts, the PostgreSQL identifier deny list, and abort-on-unexpected-diff behavior

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Record the User Story 8 baseline before the first behavior change. No user story implementation starts until this checkpoint passes.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [X] T004 Pin the PostgreSQL 18 server version and the psql, Node.js, Python, Rust, and JDBC client versions in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [X] T005 Run `cargo nextest run -p kalamdb-dialect` and `cargo nextest run -p kalamdb-postgres-wire` from `backend/` and record current correctness failures in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`; performance evidence comes from T096, not unit-test runtimes
- [X] T006 Refresh open rows in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md` against the revision being changed
- [X] T096 [US8] Create the executable PostgreSQL differential/performance harness in `benchv2/pg18/`, pin resolved dependencies/server/clients, and record workload parameters, baseline measurements, byte budgets and acceptance targets in `contracts/compatibility-matrix.md` per `contracts/execution-performance.md`; complete before behavior changes.
- [X] T103 [US1] Extend existing cache ownership in `backend/crates/kalamdb-plan-cache/src/lib.rs` with dependency-aware keys/invalidation and byte admission; test different parameter OIDs, settings, principals, RLS/grant changes, prepared invalidation, cache churn and pinned objects in `backend/crates/kalamdb-core/tests/prepared_metadata_cache.rs`. Foundational scope precedes G1; G2/G3 extend dependency epochs as they land.

**Checkpoint**: T004–T006, T096, and T103 completed. Baseline and quantitative budgets recorded. User story implementation can begin with User Story 1. T112, in the Function and V8 section, is the function baseline and must be done before function behavior changes.

---

## Phase 3: User Story 1 - Submit PostgreSQL SQL once (Priority: P1) 🎯 MVP

**Goal**: Production SQL parses once with `KalamDbDialect`, built on `PostgreSqlDialect` by `derive_dialect!` with `preserve_type_id = true`, and DataFusion plans that AST with `statement_to_plan`.

**Independent Test**: Run one query and one parameterized data change, including prepared descriptions and a retry, through HTTP SQL and a PostgreSQL connection. Confirm one syntax parse and shared metadata planning per valid revision, working execution, and a located error for malformed SQL. The dialect corpus matches upstream PostgreSQL for casts, arrays, dollar-quoted strings, nested comments, JSON operators, and quoted identifiers.

### Tests for User Story 1

- [X] T007 [P] [US1] Add a failing corpus in `backend/crates/kalamdb-dialect/tests/pg_syntax_corpus.rs` that parses casts, arrays, dollar-quoted strings, nested comments, JSON operators, and quoted identifiers with `KalamDbDialect` and asserts the same AST as `PostgreSqlDialect`, including `dialect.is::<PostgreSqlDialect>()`
- [X] T008 [P] [US1] Add failing colliding-prefix tests in `backend/crates/kalamdb-dialect/tests/kalam_extensions.rs` for `CREATE USER` as identity, rejection of `CREATE USER TABLE`, and topic `CREATE TRIGGER` / `CREATE PROCEDURE` forms that must not capture ordinary PostgreSQL statements
- [X] T009 [P] [US1] Add a failing production-path guard in `backend/crates/kalamdb-dialect/tests/no_direct_dialect.rs` that rejects `PostgreSqlDialect {}` and `GenericDialect {}` outside the exception list in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md`
- [X] T010 [P] [US1] Add a failing parse-once test in `backend/crates/kalamdb-core/tests/statement_plan_once.rs` that plans one `SELECT` and one parameterized DML, plus a retry, through `SessionState::statement_to_plan` on `datafusion_sql::parser::Statement::Statement` and fails if `session.sql` runs

All T014–T024 migrations consume a shared token cursor, AST, or explicit distinct expression source unit. Replacing independent parsing calls with repeated `KalamParser::parse(sql)` is not completion. T010 instruments tokenization, syntax parsing and planning separately; permitted reanalysis must not fail a one-plan-ever assertion.

### Implementation for User Story 1

- [X] T011 [US1] Replace the type alias in `backend/crates/kalamdb-dialect/src/dialect.rs` with `derive_dialect!(KalamDbDialect, PostgreSqlDialect, preserve_type_id = true)` and delete the alias. The derive lives in `src/kalam_dialect.rs` because sqlparser_derive 0.5.0 reads `src/dialect/mod.rs` from the invoking crate; `build.rs` refreshes that snapshot from the locked sqlparser trait
- [X] T012 [US1] Add `KalamParser` in `backend/crates/kalamdb-dialect/src/parser.rs` using `Tokenizer` with `KalamDbDialect` and `Parser::with_tokens_with_locations`, export it from `backend/crates/kalamdb-dialect/src/lib.rs`, and leave `Dialect::parse_statement` as `None` for PostgreSQL statements
- [X] T013 [US1] Route classification through `KalamParser` and delete the local `PostgreSqlDialect` construction in `backend/crates/kalamdb-dialect/src/classifier/engine/core.rs`
- [X] T014 [P] [US1] Parse views with `KalamParser` and delete the old dialect value in `backend/crates/kalamdb-dialect/src/ddl/create_view.rs`
- [X] T015 [P] [US1] Parse `ALTER TABLE` with `KalamParser` and delete the old dialect value in `backend/crates/kalamdb-dialect/src/ddl/alter_table.rs`
- [X] T016 [P] [US1] Parse user commands with `KalamParser` and delete the old dialect value in `backend/crates/kalamdb-dialect/src/ddl/user_commands.rs`
- [X] T017 [P] [US1] Parse comments with `KalamParser` and delete the `PostgreSqlDialect` value in `backend/crates/kalamdb-dialect/src/ddl/comment.rs`
- [X] T018 [P] [US1] Parse schedules with `KalamParser` and delete the `PostgreSqlDialect` tokenizer in `backend/crates/kalamdb-dialect/src/ddl/create_schedule.rs`
- [X] T019 [P] [US1] Parse policies with `KalamParser` and delete the `PostgreSqlDialect` value in `backend/crates/kalamdb-dialect/src/ddl/policy_commands/mod.rs`
- [X] T020 [P] [US1] Parse subscribe commands with `KalamParser` and delete direct dialect construction in `backend/crates/kalamdb-dialect/src/ddl/subscribe_commands.rs`
- [X] T021 [P] [US1] Parse table DDL with `KalamParser` and delete the `PostgreSqlDialect` value in `backend/crates/kalamdb-dialect/src/ddl/create_table/parser.rs`
- [X] T022 [P] [US1] Parse row-level-security expressions with `KalamDbDialect` in `backend/crates/kalamdb-rls/src/policy_compiler.rs`
- [X] T023 [P] [US1] Parse schema-diff SQL with `KalamDbDialect` in `cli/crates/kalam-schema-diff/src/parser/mod.rs` and `cli/crates/kalam-schema-diff/src/parser/create_policy.rs`
- [X] T024 [P] [US1] Parse row filters with `KalamDbDialect` in `backend/crates/kalamdb-row-filter/src/parser.rs`
- [X] T025 [US1] Replace `GenericDialect` and `session.sql`, including retries, in `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs` with `SessionState::statement_to_plan` on the shared `datafusion_sql::parser::Statement::Statement`
- [X] T026 [US1] Delete the second `create_logical_plan` on SQL text in `backend/crates/kalamdb-postgres-wire/src/statement.rs` so prepared parameter and result metadata come from the executor plan
- [X] T027 [US1] Keep `SQL_PARSER_DIALECT` as the defensive `postgresql` default and reject selection of another dialect in `backend/crates/kalamdb-core/src/sql/datafusion_session.rs`
- [X] T028 [US1] Mark the dialect-alias and direct-dialect rows done, and delete any parser helper that only served the old path, in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md`

**Checkpoint**: User Story 1 passes `specs/037-pg18-compatibility/quickstart.md` Phase B. Stop before schema or type migration.

---

## Phase 4: User Story 2 - Use PostgreSQL schemas (Priority: P1)

**Goal**: One schema identity replaces the product namespace, through the rewrite program, after the persisted-format decision. `pg_namespace` and its columns stay.

**Independent Test**: On KalamDB and PostgreSQL 18, run the same fixture: connect, `CREATE SCHEMA`, set `search_path`, create an enum, a composite, and an ordinary table, bind parameters, change data in a transaction, inspect catalogs, then alter, move, and drop objects. Repeat lookup after restart.

### Tests for User Story 2

- [X] T029 [P] [US2] Add a failing schema fixture in `backend/crates/kalamdb-dialect/tests/schema_ddl.rs` for `CREATE SCHEMA`, ordinary `CREATE TABLE`, rejection of `CREATE NAMESPACE` as a permanent alias, and `CREATE USER` remaining identity
- [X] T030 [P] [US2] Add a failing search-path test in `backend/crates/kalamdb-session/src/schema_search_path.rs` for `"$user", public`, `current_schema`, `pg_catalog`, and temporary schemas

### Implementation for User Story 2

- [X] T031 [US2] Write the persisted-format decision for stored keys, Raft, protocol, config, and client contracts in `specs/037-pg18-compatibility/contracts/schema-migration.md` before any stored bytes change
- [X] T099 [US2] Design and prove stable schema/table identities with separate quoted-aware names using `backend/crates/kalamdb-commons/src/models/ids/namespace_id.rs`, `table_id.rs` and `contracts/schema-migration.md`; test rename/move/restart, case-distinct quoted names, safe physical keys, ordinary Shared defaults, and legacy default/public mapping before T032.
- [X] T032 [US2] Dry-run `scripts/pg18-rewrite/rewrite.sh --batch backend/crates/kalamdb-commons --exceptions specs/037-pg18-compatibility/contracts/schema-migration.md`, then, after T099 proves stable identity and quoted-name resolution, replace `NamespaceId` with `SchemaId` in `backend/crates/kalamdb-commons/src/models/ids/namespace_id.rs` and delete the old name outside the deny list
- [X] T033 [US2] Move the schema catalog in place and delete the namespace module in `backend/crates/kalamdb-system/src/providers/namespaces/namespaces_provider.rs`
- [X] T034 [US2] Move schema DDL handlers in place and delete the namespace handlers, starting at `backend/crates/kalamdb-handlers/crates/ddl/src/namespace/mod.rs`
- [X] T035 [US2] Implement schema resolution, including `"$user", public`, in `backend/crates/kalamdb-session/src/schema_search_path.rs` and remove the duplicate namespace lookup it replaces
- [X] T036 [US2] Run `scripts/pg18-rewrite/rewrite.sh --batch backend/crates/kalamdb-core --exceptions specs/037-pg18-compatibility/contracts/schema-migration.md` and hand-fix only non-mechanical behavior in that crate
- [X] T037 [P] [US2] Run `scripts/pg18-rewrite/rewrite.sh --batch backend/crates/kalamdb-postgres-wire --exceptions specs/037-pg18-compatibility/contracts/schema-migration.md` and hand-fix only non-mechanical behavior in that crate
- [X] T038 [P] [US2] Run `scripts/pg18-rewrite/rewrite.sh --batch cli --exceptions specs/037-pg18-compatibility/contracts/schema-migration.md` and hand-fix only non-mechanical behavior in that tree
- [X] T039 [P] [US2] Run `scripts/pg18-rewrite/rewrite.sh --batch ui --exceptions specs/037-pg18-compatibility/contracts/schema-migration.md` and hand-fix only non-mechanical behavior in that tree
- [ ] T040 [US2] Regenerate SDK contracts from the schema identity and do not hand-edit generated trees under `link/sdks/`
- [ ] T041 [US2] Close the namespace leftover rows in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md` after a deny-list search shows `pg_namespace`, `nspname`, `nspowner`, `nspacl`, `typnamespace`, `pronamespace`, and `relnamespace` unchanged

**Checkpoint**: T099 identity/quoted-name tests plus schema create, ordinary table create, lookup, and restart pass `specs/037-pg18-compatibility/quickstart.md` Phase C. Public schema DDL accepts `CREATE SCHEMA`, `ALTER SCHEMA`, `DROP SCHEMA`, `SHOW SCHEMAS`, and `SET SCHEMA`. `T040` and `T041` stay open: SDK contracts were not regenerated, and the deny-listed `pg_namespace` names stay.

---

## Phase 5: User Story 3 - Keep PostgreSQL type and routine meaning (Priority: P1)

**Goal**: `LogicalTypeRef` is the only type meaning. Arrow layouts and wire OIDs are derived. `ColumnDefinition`'s parallel type fields go away. No second PostgreSQL type enum.

**Independent Test**: Approve the type contract, then prove `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array through planning, active storage, flushed Parquet, prepared descriptions, and text and binary results against PostgreSQL 18.

### Tests for User Story 3

- [X] T042 [P] [US3] Add a failing vertical-slice test in `backend/crates/kalamdb-core/tests/pg_type_slice.rs` for `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array through plan, storage, and text and binary results
- [X] T043 [P] [US3] Add modifier fixtures in `backend/crates/kalamdb-dialect/tests/routine_types.rs` and effective-signature tests in `backend/crates/kalamdb-core/tests/pg18_routine_ddl.rs`: function binding discards parenthesized type modifiers as FR-073 requires while the shared AST may preserve syntax, composite attributes retain applicable modifiers, and `PROCEDURE ... RETURNS` is rejected or explicitly migrated, never silently reinterpreted

### Implementation for User Story 3

- [X] T044 [US3] Record the design-review choice in `specs/037-pg18-compatibility/contracts/type-system.md` before broad type migration, comparing a PostgreSQL type catalog alone, Arrow types alone, and one logical type with derived adapters
- [X] T045 [US3] Extend `LogicalTypeRef` in `backend/crates/kalamdb-commons/src/models/datatypes/logical_type_ref.rs` so builtin, named, array, and composite forms have one owner, and do not add a new type enum beside `KalamDataType` (optimized: `LogicalTypeRef::Builtin(KalamDataType)`, `Jsonb`, and `TimestampTz` as zero-overhead variants without redundant per-builtin `BuiltinModifier`, plus `sql_name_cow` / zero-alloc `Display`)
- [X] T046 [US3] Make `ColumnDefinition` in `backend/crates/kalamdb-commons/src/models/schemas/column_definition.rs` store one `LogicalTypeRef` and delete `data_type`, `named_type_id`, and `is_array` as parallel authorities (optimized: removed redundant `element_nullable` field in favor of `LogicalTypeRef::List`, in-place `set_*` mutations via `mem::replace`, single-allocation `to_sql()`, and zero-clone `TableDefinitionSerializeRepr`)
- [X] T047 [US3] Resolve named types and modifiers through `TypeRegistry` in `backend/crates/kalamdb-core/src/schema_registry/type_registry/mod.rs`, with a layout cache per schema revision
- [ ] T048 [US3] Derive pgwire OIDs and codecs from `LogicalTypeRef` in `backend/crates/kalamdb-postgres-wire/src/statement.rs` and delete the wire-owned type map
- [X] T049 [US3] Keep the decimal128 fast path for bounded `numeric` and write the wide or unconstrained `numeric` decision in `specs/037-pg18-compatibility/contracts/type-system.md` instead of renaming `DECIMAL` to `NUMERIC` only
- [ ] T100 [US3] Add stable enum member identity, complete array value metadata, nested type revisions, numeric edge cases and explicit JSONB/temporal codecs to the T045–T049 slice; put differential fixtures under `backend/crates/kalamdb-core/tests/pg18_type_semantics.rs` and record unsupported cases in `contracts/type-system.md`.
- [ ] T050 [US3] Consume PostgreSQL function/procedure language and body forms through the shared AST in `backend/crates/kalamdb-dialect/src/parser/` and typed DDL adapters; preserve SQL/plpgsql semantics and keep `FILE`, `EMBEDDING`, and JS/TS languages as documented Kalam extensions. `file` and `embedding` follow T154. T125/T128/T132 supply the quoting, identity and handler acceptance contract; grammar alone does not close T110
- [ ] T154 [US3] Install the required `file` extension and `pg_catalog.embedding` once during database creation, as specified in `specs/037-pg18-compatibility/contracts/type-system.md`. Implement `file_put`, `file_read`, and `file_delete` in Rust; do not re-run `CREATE TYPE` on server start, do not keep a `FILE` or `EMBEDDING` keyword as a parallel logical type, and do not generate project routines. Reject a caller-constructed reference and reject `DROP`/`ALTER` of these system objects. Keep blob lifecycle in the filestore. Keep `embedding(n)` dimension `1..=8192`. Merge the extension into the contract snapshot in `cli/src/workflow/schema/load.rs` and emit the type plus the three functions from `typescript/schema.rs`, `typescript/client.rs`, `dart/`, and `rust.rs`. Make `@kalamdb/client` `FileRef` wrap that generated shape and delete `FileRefData` plus the private download API in `link/sdks/typescript/client/src/file_ref.ts`. Add `backend/crates/kalamdb-core/tests/pg18_extension_types.rs` proving the type and functions are visible, drop fails, they survive restart, and a forged file reference fails. Extend `cli/src/workflow/schema/typescript/mod.rs` so a generated column completes `id`, `sub`, `name`, `size`, `mime`, `sha256`, and `shard`, and the generated client calls `file_put`, `file_read`, and `file_delete`.
- [ ] T110 [US3] Add executable SQL/plpgsql routine-body lifecycle and dependency tests in `backend/crates/kalamdb-core/tests/pg18_routines.rs`; implement required runtime semantics through existing routine owners or retain an explicit blocking gap. T050 grammar acceptance alone cannot verify a language.
- [ ] T124 [US3] Add domain and type-form coverage for `CREATE`, `ALTER`, and `DROP DOMAIN` and for composite and enum `CREATE`, `ALTER`, and `DROP TYPE` in `backend/crates/kalamdb-dialect/tests/type_forms.rs` and `backend/crates/kalamdb-core/tests/pg18_domains.rs`. Inventory range, base, and shell type forms in `specs/037-pg18-compatibility/contracts/type-system.md`: implement each form the matrix requires, and record every other form as an explicit gap before the User Story 3 checkpoint (FR-077).
- [ ] T051 [US3] Close the parallel type-field rows in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md` and delete the unused mapping helpers those fields required

### Row storage v2 for User Story 3

Contract: `specs/037-pg18-compatibility/contracts/row-storage.md`. A format switch is conditional on semantic, performance and upgrade evidence. T097–T102 precede activation. The v1 decoder stays for the supported recovery window.

**Runtime v1 RocksDB row, Raft log, secondary-index, and stream-log codec optimizations landed (`backend/crates/kalamdb-serialization/`, `kalamdb-raft/`, `kalamdb-streams/`, `kalamdb-core/`)**:
- Single-allocation envelope + row encoding (`begin_envelope` / `finish_envelope` + `encode_fields_indexed` writing payload-relative offsets in place without double-buffering).
- Zero-clone direct row decoding in `decode_row_body` (eliminating per-row `DecodedSlots` `Vec<ScalarValue>` allocation and `ScalarValue::clone()` across `slots_to_named_row`).
- Zero-allocation offset-table slicing in `decode_selected_indexed` for projected RocksDB point reads.
- Direct columnar push in `decode_payloads_to_arrow_batch` (`decode_row_body_into_columns`) without intermediate per-row `Vec<ScalarValue>`.
- Direct-indexed O(N) `build_storage_schema` in `row/from_table.rs` without intermediate sorted `Vec` + `BTreeMap`.
- Memoized `Arc<StorageSchema>` in `CachedTableData` (`OnceLock<Arc<StorageSchema>>` + cached `resolved_arrow` fallback) so Raft `ordinal_dml::encode_insert_rows` and `decode_insert_rows` no longer rebuild `StorageSchema` on every proposal and apply.
- Eliminated full `Vec<Row>` clones in `ProviderUserDataApplier::insert` / `ProviderSharedDataApplier::insert` -> `DmlExecutor::insert_{user,shared}_data_with_versions`.
- Added packed binary codecs (`0xA4` / `0xA5`) for `UserDataCommand::Delete` and `SharedDataCommand::Delete` in `kalamdb-raft/src/codec/command_codec.rs` and single-buffer `encode_stored_entry` in `kalamdb-raft/src/storage/raft_store.rs`.
- Replaced FlexBuffers with compact binary framing (`schema_version = 2` with `schema_version = 1` fallback) for secondary-index PK lists (`encode_string_list` / `decode_string_list`) and stream log frames (`append_stream_put_frame`, `append_stream_delete_frame`, `decode_raw_stream_record`), plus read-buffer reuse and lazy reverse row decoding in `FileStreamLogStore`.

- [ ] T097 [US3] Implement and test version/flag rejection, read-old/write-old deployment stage, durable format activation and downgrade/recovery policy in `backend/crates/kalamdb-serialization/src/object/envelope.rs` and existing Raft/startup compatibility boundaries; record crash/failover/backup fixtures in `contracts/row-storage.md` before T090 is enabled.
- [ ] T098 [US3] Prototype direct builders, projected decode and view-versus-compacted ownership in `backend/crates/kalamdb-serialization/src/row/batch.rs` and `benchv2/pg18/`; measure selective LIMIT, wide rows, null-heavy rows and slow portals before choosing T091/T092 buffer strategy.
- [ ] T101 [US3] Implement historical missing values, atomic conversion/backfill, history publication, revision exhaustion and bounded layout resolution in the schema registry and `backend/crates/kalamdb-tables/src/row_codec.rs`; add default-change, failed USING, drop/re-add, old snapshot and restore tests in `backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs` before T089/T090 acceptance.
- [ ] T102 [US3] Add malformed-payload/truncation/overflow and bounded-expansion tests to `backend/crates/kalamdb-serialization/src/row/`; verify buffer lifetime, constraints, aborted writes and flush tombstone/version retention in table/flush tests before T093/T094 acceptance.
- [ ] T086 [P] [US3] Add failing compatibility tests in `backend/crates/kalamdb-serialization/src/row/mod.rs`: a v1 row decodes under v2 code, and a row written under a two-column schema decodes after `ADD COLUMN`, `DROP COLUMN`, `RENAME COLUMN`, and `ALTER COLUMN TYPE` through per-version schemas
- [ ] T087 [P] [US3] Extend `backend/crates/kalamdb-serialization/benches/codec.rs` with a text-heavy row and a payloads-to-`RecordBatch` case, and record the v1 numbers in seconds before any layout change
- [ ] T088 [US3] Add the per-version layout (fixed permutation, constant offsets, fixed length, variable column list) to `StorageSchema` in `backend/crates/kalamdb-serialization/src/row/schema.rs`, computed once per version
- [ ] T089 [US3] Keep every `StorageSchema` version per table in the catalog and resolve a row's envelope `schema_version` to it in `backend/crates/kalamdb-tables/src/row_codec.rs`, with DDL-time missing values, bounded RAM caches, nested type dependencies, recovery reachability and revision-overflow rejection (T101)
- [ ] T090 [US3] Implement the v2 body encoder and `RowView<'a>` decoder behind a new envelope flag in `backend/crates/kalamdb-serialization/src/row/encode.rs`, `decode.rs`, and `object/envelope.rs`: flags, u16 count, null bitmap, packed fixed region, per-row-width end offsets, no redundant scalar tags, full boundary validation on read/write, explicit tombstone/nested layouts and checked size limits; v2 writers remain disabled until T097 activation checks pass
- [ ] T091 [US3] Replace `decode_payloads_to_arrow_batch` in `backend/crates/kalamdb-serialization/src/row/batch.rs` with a `RowView` path that writes primitives into Arrow builders and text, bytea, and jsonb into `GenericByteViewBuilder` blocks, and adapt the hot-row Arrow schema in `backend/crates/kalamdb-serialization/src/row/from_table.rs` only after kernel/Parquet compatibility and retained-buffer measurements pass T098
- [ ] T092 [US3] Implement bounded owned chunks or per-value owners in `backend/crates/kalamdb-store/src/backends/rocksdb/backend.rs` using T098 measurements; avoid collect-then-copy, account retained bytes, and release on cancellation/backpressure
- [ ] T093 [US3] Encode DML from Arrow arrays into a pre-sized reusable buffer in `backend/crates/kalamdb-tables/src/utils/datafusion_dml.rs` preserving all DML validation/atomicity and buffer ownership (T102), then delete `record_batches_to_rows` and the map-lookup encoder
- [ ] T094 [US3] Make flush call the shared payload-to-`RecordBatch` function in `backend/crates/kalamdb-flush/src/flush/scope_writer.rs` with flush-specific version/tombstone retention (T102), and delete `rows_into_arrow_batch` from `Row` when no caller remains
- [ ] T095 [US3] Run `cargo bench --profile dev -p kalamdb-serialization --bench codec`, confirm T087/T096 workload-specific time, size, allocation and retained-memory gates, close the row-storage rows in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md`, and update `docs/architecture/` for the v2 body and schema history

**Checkpoint**: The vertical slice passes `specs/037-pg18-compatibility/quickstart.md` Phase D, including the codec bench gate. T124 is done, or every unimplemented type form is an explicit matrix gap. Unknown types are not advertised as text.

---

## Phase 6: User Story 4 - Match results, errors, and transactions (Priority: P2)

**Goal**: The ordinary application slice matches PostgreSQL 18 for results, types, SQLSTATE, permissions, commit, and rollback. VersionId ordering and one persisted owner per shared table stay.

**Independent Test**: Run the ordinary fixture on PostgreSQL 18 and KalamDB. Then run T104/T105 adversarial transaction and DDL cases. One rollback and one concurrent write are smoke tests only.

### Tests for User Story 4

- [ ] T052 [P] [US4] Add the ordinary differential fixture in `backend/crates/kalamdb-core/tests/pg18_execution_slice.rs` comparing results, types, SQLSTATE, permissions, and commit or rollback using T096 oracle normalization rules
- [ ] T053 [P] [US4] Add the rollback and concurrent-write cases in `backend/crates/kalamdb-core/tests/pg18_transaction_cases.rs`

### Implementation for User Story 4

- [ ] T054 [US4] Return PostgreSQL SQLSTATE and result metadata from the shared plan in `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs`
- [ ] T055 [US4] Preserve VersionId ordering and the shared-table owner rule, and record the cross-group transaction gap, in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [ ] T056 [US4] Delete any execution fallback that reports success for a rolled-back statement in `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs` and note the removed path in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md`
- [ ] T104 [US4] Expand `backend/crates/kalamdb-core/tests/pg18_transaction_cases.rs` and existing transaction owners to implement every required isolation/savepoint/constraint/failed-state row in `contracts/execution-performance.md`; unsupported behavior fails explicitly and cannot close G4 without a product scope decision.
- [ ] T105 [US4] Implement safe pre-execution replan versus unsafe mutation-retry boundaries in `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs`; test ambiguous commit, volatile effects, transactional DDL and aborted/committed live-event visibility in `backend/crates/kalamdb-core/tests/pg18_transaction_cases.rs` while preserving domain-local VersionId semantics.
- [ ] T123 [US4] Record in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md` that a schema rename, column type change, or rolled-back schema change does not keep an existing subscription connected: the client disconnects the websocket and connects again, and the new subscription and user-scoped reads use the schema that committed (FR-049, FR-090). Add the matching test in `backend/crates/kalamdb-core/tests/pg18_subscription_schema.rs` using the existing subscription owners. Do not implement a subscription that stays open across the schema change.

**Checkpoint**: G4 requires T104, T105, T123, and all required rows of `contracts/execution-performance.md`, not just T052/T053 smoke cases; unresolved cross-domain limitations remain explicit gaps.

---

## Phase 7: User Story 5 - See truthful catalogs (Priority: P2)

**Goal**: `pg_catalog` and `information_schema` are shared projections of authoritative records. Empty wire probes disappear as real rows replace them. Column types match PostgreSQL 18.

**Independent Test**: Create schema, table, constraint, index, enum, composite, array, domain, function, and procedure fixtures. Compare catalog rows with PostgreSQL 18 through HTTP SQL and the wire. Then rename, drop, roll back, and restart.

### Tests for User Story 5

- [ ] T057 [P] [US5] Add catalog comparison tests in `backend/crates/kalamdb-views/tests/pg_catalog_slice.rs` for schema, table, enum, and routine rows against the PostgreSQL 18 shapes in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [ ] T058 [P] [US5] Add a failing empty-probe test in `backend/crates/kalamdb-postgres-wire/src/client_catalog/empty_tables.rs` that fails when a stub is still served for an object the caller can see

### Implementation for User Story 5

- [ ] T059 [US5] Project `pg_catalog` and `information_schema` from system records in `backend/crates/kalamdb-views/` and share those providers with `backend/crates/kalamdb-postgres-wire/src/client_catalog/mod.rs`
- [ ] T060 [US5] Delete replaced stubs from `backend/crates/kalamdb-postgres-wire/src/client_catalog/empty_tables.rs` and keep an empty result only when the caller truly sees nothing
- [ ] T061 [US5] Match PostgreSQL 18 catalog column types, and do not expose credentials through `pg_authid`, in `backend/crates/kalamdb-views/`
- [ ] T062 [US5] Close the empty-catalog rows in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md`

**Checkpoint**: Both SQL entry points return the same catalog rows for the fixture, including after restart.

---

## Phase 8: User Story 6 - Connect with PostgreSQL clients (Priority: P2)

**Goal**: Pinned clients use the existing wire server. Prepared describe, bind, and encode share the User Story 3 descriptors.

**Independent Test**: With pinned psql and the selected Node.js, Python, Rust, and JDBC drivers, pass connection, query, prepared statement, transaction, cancellation, bulk transfer, and reconnect. Parameter and result descriptions match execution.

### Tests for User Story 6

- [ ] T063 [P] [US6] Add the pinned-client checklist and expected versions in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [ ] T064 [P] [US6] Add wire extended-query coverage for describe, bind, and execute in `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs` using the shared type descriptors

### Implementation for User Story 6

- [ ] T065 [US6] Serve prepared descriptions from the shared plan in `backend/crates/kalamdb-postgres-wire/src/statement.rs` and delete the separate inference path
- [ ] T066 [US6] Cover cancellation, bulk transfer, and reconnect on the existing wire connection in `backend/crates/kalamdb-postgres-wire/src/connection.rs` without a second planner
- [ ] T067 [US6] Record client results and any explicit gap in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [ ] T106 [US6] Expand `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs` and connection state handling for error-to-Sync, ReadyForQuery state, portals, pipelining, COPY, simple-query batch atomicity, invalid binds, cancellation and bounded output; record every required transition in `contracts/compatibility-matrix.md`.
- [ ] T107 [US6] Implement/test HTTP versus wire session isolation and byte limits in existing API/session/connection owners; record session-handle, SET/SET LOCAL, transaction and pool-reuse behavior in `contracts/execution-performance.md`, including cross-user rejection tests.

**Checkpoint**: T106/T107 and pinned psql plus one driver from each selected family pass, including protocol error recovery and bounded session state.

---

## Phase 9: User Story 7 - Migrate and operate with familiar tools (Priority: P2)

**Goal**: A representative application can move schema, data, sequences, constraints, and indexes, then run against the shared catalogs.

**Independent Test**: Point pinned psql, a driver, an ORM, and a GUI at the catalogs. Load the migration fixture and compare schema, typed data contents, sequence state, constraints and indexes with PostgreSQL 18.

### Tests for User Story 7

- [ ] T068 [P] [US7] Capture the ORM and GUI introspection statements in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [ ] T069 [P] [US7] Add the migration fixture and typed-content, schema, sequence-state and row-count comparisons in `cli/tests/pg18_migration_fixture.sql`
- [ ] T109 [US7] Extend the migration harness around `cli/tests/pg18_migration_fixture.sql` to compare typed data contents, schema/constraint/index definitions, sequence state and subsequent generated values; intentionally corrupt equal-count data to prove the comparator catches it.

### Implementation for User Story 7

- [ ] T070 [US7] Accept the captured introspection statements through the shared catalog providers in `backend/crates/kalamdb-views/`
- [ ] T071 [US7] Load schema, data, sequences, constraints, and indexes for the fixture via `cli/` and compare typed contents, schema, constraints, indexes, sequence state and counts with the PostgreSQL 18 source (T109)
- [ ] T072 [US7] Record tooling gaps as explicit matrix rows in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md` rather than silent skips

**Checkpoint**: The representative application runs, and typed contents, schema, sequence state and row counts match the source.

---

## Phase 10: User Story 8 - Prove compatibility without losing KalamDB behavior (Priority: P1)

**Goal**: Each behavior change carries a new differential test, a migrated existing test, the revision, and any skip reason. Final sign-off waits until User Stories 4 through 7 have evidence.

**Independent Test**: The Phase 2 baseline already pinned versions and recorded failures. For a later change, show the differential test, the migrated test, the revision, and any skip reason. At final sign-off, show the full regression run, deletion review, budgets, and the unchanged developer journey.

### Tests for User Story 8

- [ ] T073 [P] [US8] Add the evidence checklist in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md` requiring a differential test, a migrated existing test, a revision, and a skip reason for each behavior change
- [ ] T074 [P] [US8] Add a rewrite dry-run for one test directory in `scripts/pg18-rewrite/rewrite.sh` that changes no deny-listed PostgreSQL identifiers

### Implementation for User Story 8

- [ ] T075 [US8] Attach User Story 1 through 3 evidence rows in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [ ] T076 [US8] Run test and fixture renames one directory per invocation of `scripts/pg18-rewrite/rewrite.sh`, with exceptions in `specs/037-pg18-compatibility/contracts/schema-migration.md`, and do not hand-edit the whole test tree
- [ ] T108 [US8] Use `benchv2/pg18/` profiles and plan evidence to improve existing DataFusion provider projection/predicate pushdown, point/index/range reads, statistics, spill/admission and catalog batching; preserve version/deletion semantics and record base/candidate/PG ratios before final acceptance. No speculative second execution engine.
- [ ] T111 [US8] Update canonical SDK docs in `../KalamSite/content/sdk/**`, user-facing skill content in `../kalamdb-skills` and generated mirrors alongside the corresponding behavior batches; final release adds `docs/releases/<version>.md` and its README link. Verify these updates before T077/T080.
- [ ] T077 [US8] After User Stories 4 through 7, run `scripts/test-all.sh` from the repo root and record the revision, deletion review, and budgets in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`

**Checkpoint**: Final sign-off shows the full run, a clean ledger, and budgets inside the Phase 2 baseline.

---

## Phase 11: User Story 9 - Install, upgrade, and recover a release (Priority: P3)

**Goal**: Release artifacts support a clean disposable project and an upgrade, backup, restore, restart, and interrupted-operation rehearsal. The published statement matches the matrix.

**Independent Test**: Using the actual release artifacts, complete that journey and confirm the compatibility statement matches `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`.

### Tests for User Story 9

- [ ] T078 [P] [US9] Add the release rehearsal checklist in `docs/releases/README.md` for clean install, upgrade, backup, restore, restart, and interrupted operation

### Implementation for User Story 9

- [ ] T079 [US9] Write the compatibility statement so it matches the matrix in `docs/pages/kalamdb-0.8-epics.md`
- [ ] T080 [US9] Rehearse upgrade, backup, restore, restart, and interrupted operation against the release artifacts and record the result in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`
- [ ] T081 [US9] Rebuild the CLI with `cargo build -p kalam-cli --bin kalam` and run the smoke command in `specs/037-pg18-compatibility/quickstart.md` against a fresh server

**Checkpoint**: The published statement and the rehearsal notes describe the same matrix.

---

## Phase 12: Polish & Cross-Cutting Concerns

**Purpose**: Docs and the final quickstart pass after the stories above.

- [ ] T082 [P] Update the dialect decision so it names `derive_dialect!` and `preserve_type_id` in `specs/036-postgresql-sql-dialect/research.md`
- [ ] T083 [P] Update the architecture note for one parse and `statement_to_plan` in `docs/architecture/decisions/adr-012-sqlparser-integration.md`
- [ ] T084 Confirm every replaced path in `specs/037-pg18-compatibility/contracts/cleanup-ledger.md` is closed or has an owner and a removal milestone
- [ ] T085 Run the gate commands in `specs/037-pg18-compatibility/quickstart.md` and record the revision beside each one

---

## Function and V8 integration tasks

User-requested addition; contract: `contracts/functions-v8.md`. These run with G3 and the relevant G4/verification gates, not after release. Each implementation includes its failing behavior/ownership tests first. T112 is baseline work; T113 precedes signature/transport changes. New fast paths remain disabled until their T116/T120 lifetime and accounting checks pass.

- [ ] T112 [US8] Extend `backend/crates/kalamdb-functions/benches/v8_runtime.rs` and `benchv2/pg18/` with cold/warm calls, Arrow result conversion, named arguments, query→nested call→return, binary/text/nested values, sparse/full reads and copied-byte instrumentation; record rusty_v8/build/sandbox flags and existing admission limits before changes in `contracts/functions-v8.md`.
- [ ] T113 [US3] Define the next function ABI and lossless logical-type-derived call descriptors in `backend/crates/kalamdb-functions/src/abi/` and `backend/crates/kalamdb-functions-host/src/spec.rs`; finalize read-view/materialization/transfer mappings, NULL/undefined, exact numeric/temporal types, legacy ABI migration and generated declarations in `contracts/functions-v8.md` before changing RoutineValue. Keep each new model in its own file.
- [ ] T114 [US3] Evolve `backend/crates/kalamdb-functions/src/abi/value.rs` and `backend/crates/kalamdb-core/src/functions/{bind,convert,executor}.rs` to retain typed Arrow cell/batch owners, bind named arguments by cached ordinals and materialize ordinary JS objects directly; remove replaced per-row scalar arrays, repeated JSON packing/key probes and redundant transfer caches after caller migration. Preserve the small ScalarRows fast path and explicit conversion errors.
- [ ] T115 [US3] Implement invocation-owned read views and bounded lazy field access in `backend/crates/kalamdb-functions/src/abi/convert.rs` and `src/v8/`, with descriptor/projection checks, safe enumeration, explicit materialize and unforgeable generation-checked native handles; expose the versioned queryView host API through `backend/crates/kalamdb-functions-host/src/{spec,emit_js}.rs`. Test normal mutable-query compatibility before enabling views.
- [ ] T116 [US3] Implement eligible exclusive-buffer transfer and a shared memory-lease/accounting policy in `backend/crates/kalamdb-functions/src/v8/buffer_allocator.rs`, `src/engine/` and `src/abi/convert.rs`; prove sandbox/allocator eligibility, full-buffer authorized exposure, detach/alias safety, mutation isolation, offset/alignment correctness, owner release and explicit copy fallback. External-string sharing is optional and requires its own encoding/lifetime measurements.
- [ ] T117 [US4] Update `backend/crates/kalamdb-functions/src/v8/async_ops.rs`, `src/host/trait_host.rs` and `backend/crates/kalamdb-core/src/functions/host.rs` to forward compatible native views and returned owners without scalar/JSON rebuilding or parameter-vector deep clones; preserve child realms/immutable permission frames, common transaction/deadline and root HTTP restrictions. Test parent microtasks, invoker/definer calls and data-only rewrapping.
- [ ] T118 [US3] Extend `backend/crates/kalamdb-functions/tests/v8_regressions.rs` and core function tests with exact Int64/numeric/temporal values, negative scale, JSON null versus SQL NULL, arrays/lower bounds, enums/domains, Unicode, overflow, undefined/missing fields and cyclic/accessor objects; fix lossy or permissive ABI conversions through the canonical descriptor rather than custom per-call mappings.
- [ ] T119 [US4] Implement bounded queryBatches iteration and explicit batch-routine semantics in `backend/crates/kalamdb-functions-host/src/spec.rs`, `backend/crates/kalamdb-core/src/functions/host.rs` and `backend/crates/kalamdb-functions/src/v8/async_ops.rs`; test backpressure, early break/close, late results, await-held views, failure/cancel, row-order and volatile-function behavior without collecting full results.
- [ ] T120 [US8] Extend `backend/crates/kalamdb-functions/tests/runtime_lifecycle.rs`, `tests/v8_regressions.rs` and core integration tests for owner identity/copy counts, quotas on imported buffers, stale/forged handles, alias writes, detach/resize/shared-buffer rejection or copy, GC delay, timeout/OOM/cancel, revision change, isolate reuse and high concurrency. Verify no cross-user data/capability leakage and no uncharged retained owners.
- [ ] T121 [US3] Regenerate host/SDK function declarations from `backend/crates/kalamdb-functions-host/src/spec.rs` and CLI generators; update `../KalamSite/content/sdk/**`, `../kalamdb-skills`, mirrors and `docs/architecture/` with read-view, materialize, transfer and lifetime examples. Add generated input/output type checks and a disposable-project query→function→nested-call journey; do not hand-edit generated trees.
- [ ] T122 [US8] Run the function microbenchmarks and end-to-end `benchv2/pg18/` workloads from T112, document exact copy/allocation/CPU/latency/native-and-V8 memory results and cleanup evidence in `contracts/compatibility-matrix.md`/`contracts/cleanup-ledger.md`; apply functions-v8.md targets and global regression budgets before enabling the new ABI by default.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies
- **Foundational (Phase 2)**: Depends on Setup; includes T096 and T112 before their respective behavior changes. Blocks every user story implementation
- **User Story 1 (Phase 3)**: Depends on Foundational. No dependency on later stories
- **User Story 2 (Phase 4)**: Depends on G1, T003 and T031; T099 precedes T032 and semantic identity migration precedes broad renaming
- **User Story 3 (Phase 5)**: Depends on G1 and G2 contracts. The vertical slice waits for T044, T100, T110, and T124. T097 is designed before T090 and tested before writer activation. T098 informs T091/T092; T101 gates T089/T090; T102 gates T093/T094. T095 is conditional format acceptance, not an unconditional rewrite. Function transport additionally follows T113→T114→T115, with T116/T120 safety checks before enabling fast paths and T118 type evidence; it does not wait for v2 disk activation
- **User Story 4 (Phase 6)**: Depends on G1/G2/G3 slices; closure additionally requires T104, T105, T123, and the relevant T117/T119 nested/streaming semantics
- **User Story 5 (Phase 7)**: Depends on User Stories 2 and 3
- **User Story 6 (Phase 8)**: Depends on User Stories 1 and 3. Catalog-backed client checks also depend on User Story 5; final acceptance requires T106/T107 and relevant G4 state behavior
- **User Story 7 (Phase 9)**: Depends on User Stories 4, 5, and 6
- **User Story 8 (Phase 10)**: T073 and T074 can start after Foundational. T075 needs User Stories 1 through 3. T077 needs User Stories 4 through 7, T108/T122 performance evidence, T120 runtime safety evidence and T111/T121 documentation evidence
- **User Story 9 (Phase 11)**: Depends on T077
- **Polish (Phase 12)**: Depends on the stories being closed

### User Story Dependencies

- **User Story 1 (P1)**: First behavior change. MVP
- **User Story 2 (P1)**: Depends on G1, the rewrite script, T031 and T099; its narrow gate is independently testable after these prerequisites
- **User Story 3 (P1)**: Independently testable on the vertical slice before broad migration
- **User Story 4 (P2)**: Depends on the type slice for result types
- **User Story 5 (P2)**: Depends on schema identity and the type contract
- **User Story 6 (P2)**: Wire protocol can proceed beside catalogs; final client acceptance includes both
- **User Story 7 (P2)**: Depends on execution, catalogs, and clients
- **User Story 8 (P1)**: Baseline is Phase 2. Closure is after functional evidence
- **User Story 9 (P3)**: Depends on the User Story 8 final gate

### Function slice ordering

T112 measures before implementation. T113 joins the T044/T100 type design. T114 and T115 establish native owners and wrappers; develop T116 accounting/lifetime tests alongside them before enabling reuse/transfer. T117 consumes these foundations for child/return forwarding; T118 tests each mapping as it lands. T119 adds bounded iteration and opt-in batch semantics; T120 validates each safety boundary continuously and again under integrated concurrency. T121 updates generated APIs/docs with their behavior changes; T122 closes the performance/cleanup gate. T112 does not block unrelated G1 experiments once T096 is available.

The 2026-10-13 routine follow-up adds T125–T138 below. T126/T129 define identity/binding before T133 finalizes T113. T130 must prove SQL-to-V8 execution before routine acceptance; T133/T134 complement T114–T119 rather than create another transport. T136 and T138 close correctness and performance alongside T110/T120/T122. Existing completion marks remain unchanged; the follow-up cases remain required.

The CLI follow-up is T139–T153. Begin shared project parsing/diff work alongside G1/G3 and lifecycle baselines immediately; do not postpone all CLI work until release smoke. T139 feeds T140–T142/T147; T143/T144 establish applied-state authority before T145 publication; T146/T148–T150 consume that state model. T151/T152/T153 close real-project correctness/recovery/performance before release. These tasks extend T023/T038/T121/T137 without changing their existing completion marks.

### Within Each User Story

- Tests are written first and fail before the implementation that satisfies them
- Dialect identity and the parser land before call sites move
- The persisted-format note lands before stored keys change
- The type design review lands before `ColumnDefinition` drops its parallel fields
- Schema history (T089) lands before the v2 body (T090), and the v1 decoder is not deleted in this feature
- Cleanup of the replaced path is inside the story, in the ledger task that closes it

### Parallel Opportunities

- T007, T008, T009, and T010 can run together
- T014 through T024 touch different files and can run together after T012
- T037, T038, and T039 are separate rewrite batches and can run together after T032
- T042 and T043 can run together
- T086 and T087 can run together; T093 and T094 can run together after T091 and the relevant T102 failing tests, with shared codec edits integrated sequentially
- T052 and T053 can run together
- T057 and T058 can run together
- T063 and T064 can run together
- T068 and T069 can run together
- T073 and T074 can run together
- T082 and T083 can run together

---

## Parallel Example: User Story 1

```bash
# Failing tests together:
# T007 backend/crates/kalamdb-dialect/tests/pg_syntax_corpus.rs
# T008 backend/crates/kalamdb-dialect/tests/kalam_extensions.rs
# T009 backend/crates/kalamdb-dialect/tests/no_direct_dialect.rs
# T010 backend/crates/kalamdb-core/tests/statement_plan_once.rs

# After T012, call-site moves together:
# T014 create_view.rs  T015 alter_table.rs  T016 user_commands.rs
# T017 comment.rs  T018 create_schedule.rs  T019 policy_commands/mod.rs
# T020 subscribe_commands.rs  T021 create_table/parser.rs
# T022 policy_compiler.rs  T023 kalam-schema-diff  T024 row-filter parser.rs
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 and Phase 2
2. Complete Phase 3 (User Story 1)
3. Stop and validate with `specs/037-pg18-compatibility/quickstart.md` Phase B
4. Do not start the schema rename or the type migration in that slice

### Incremental Delivery

1. Setup and baseline
2. User Story 1: one dialect, one parse and shared revision-aware planning
3. User Story 2: schemas, one crate or directory per rewrite batch
4. User Story 3: one logical type, vertical slice before broad migration
5. User Stories 4 through 7: execution, catalogs, clients, tooling
6. User Story 8 closure, then User Story 9 release rehearsal

### Parallel Team Strategy

1. Finish Setup and Foundational together
2. After T012, split the User Story 1 call-site tasks across files
3. After T031, split User Story 2 rewrite batches by crate
4. Catalog work (User Story 5) and wire client work (User Story 6) can overlap once User Stories 2 and 3 have contracts

---

## Notes

- `derive_dialect!` with `preserve_type_id = true` is the PostgreSQL dialect. A type alias, a partial `impl Dialect`, or `preserve_type_id = false` is not
- User SQL is parsed with `KalamDbDialect` and planned with `statement_to_plan`. The session setting `postgresql` is only the defensive default
- Wide renames and test updates go through `scripts/pg18-rewrite/rewrite.sh`, one batch at a time
- Each story deletes the path it replaces and updates `specs/037-pg18-compatibility/contracts/cleanup-ledger.md`
- Row storage: one KOBJ envelope, row body v2 for everything tabular, FlexBuffers only for catalog structs. No new serialization crate. Views reduce copies only when measured ownership retention justifies them; primitives and other conversion boundaries still copy
- Commit after each task or logical group

## Phase 13: Routine compatibility follow-up (2026-10-13)

Scope: the current type/routine surface and typed in-process invocation. Static review found partial/contradictory coverage even though T007–T028 and T099 are marked complete. Evidence and reference behavior are in [review.md](review.md) and [routines-postgresql.md](contracts/routines-postgresql.md). These new tasks supplement pending T043/T050/T110/T124 and T112–T122; they do not reopen completed tasks or declare unrelated work converged. The two pending task descriptions above were corrected to remove contradictory requirements.

- [ ] T125 [US1] Route ordinary type/function/procedure lifecycle and CALL through the shared token/AST path in `backend/crates/kalamdb-dialect/src/{parser,classifier/engine,ddl}/`; remove the replaced language-word classification and string-splitting paths after migration. Add production-classifier/handler tests in `backend/crates/kalamdb-dialect/tests/pg18_routine_syntax.rs` for quoted/doubled names, embedded dots, reserved names, Unicode byte limits, mixed-case language identifiers, `$tag$` bodies, nested comments, AS/LANGUAGE ordering, multiword/qualified types and trailing-token rejection. Evidence: `is_kalam_procedure`, `take_ident`, `parse_procedure_body` and `DropProcedureStatement::parse` still use bespoke rules (FR-168).
- [ ] T126 [US3] Replace name-only routine identity in `backend/crates/kalamdb-commons/src/models/ids/routine_id.rs` and catalog routine/parameter owners with stable IDs plus canonical overload keys; preserve quoted components, distinguish domains and normalize aliases, exclude OUT-only types/typmods/defaults/return types/kind from uniqueness, and document persisted-key/OID/artifact migration in `contracts/routines-postgresql.md` before writes change. Test function/procedure conflicts, rename/move/restart and overload coexistence; each new model gets its own file (FR-170).
- [ ] T127 [US3] Extend T124 in `backend/crates/kalamdb-dialect/src/ddl/create_type.rs`, existing catalog-type handlers and TypeRegistry with PG composite/enum edge cases: empty definitions, empty/colon/Unicode labels, byte limits, quoted builtin-like names, collations and table row-type/name conflicts. Add `backend/crates/kalamdb-core/tests/pg18_type_declarations.rs` including NULL composite versus all-null fields, array values and domain constraints; migrate/reject non-PG IF NOT EXISTS, FROM TABLE, inline comments/constraints as specified. Evidence: current validators reject valid empty/colon forms and count characters (FR-073, FR-077–FR-079, FR-169).
- [ ] T128 [US3] Implement distinct PostgreSQL function/procedure DDL adapters and lifecycle in `backend/crates/kalamdb-dialect/src/ddl/` and `backend/crates/kalamdb-handlers/crates/ddl/src/`, using T126 identity for CREATE/REPLACE/ALTER/DROP/COMMENT/GRANT/REVOKE. Reject procedure RETURNS on the canonical surface; enforce replacement/result/name restrictions, signature ambiguity, dependencies and RESTRICT/CASCADE. Add `backend/crates/kalamdb-core/tests/pg18_routine_ddl.rs` with atomic failed replacement and invalidation tests (FR-080–FR-083, FR-170, FR-172).
- [ ] T129 [US3] Build one explicit routine binder in `backend/crates/kalamdb-core/src/functions/bind.rs` and canonical shared models for unnamed/named IN/OUT/INOUT/VARIADIC parameters, default expressions, positional/named/mixed calls, type coercion and ambiguous overloads. Test omitted versus NULL, procedure OUT placeholders without expression evaluation, quoted parameter names and a single composite/JSON argument whose keys match parameters in `backend/crates/kalamdb-core/tests/pg18_routine_binding.rs`; derive maps from immutable descriptors, not repeated catalog scans or JSON shape probes (FR-171).
- [ ] T130 [US4] Integrate resolved routines with existing DataFusion expression, scalar/set/table and CALL execution owners in `backend/crates/kalamdb-core/src/sql/` and `src/functions/`. Prove the locked DataFusion extension mechanism can drive bounded async V8/host work without blocking query workers, deadlocking nested SQL, making an isolate per row or eagerly collecting sets. Add `backend/crates/kalamdb-core/tests/pg18_js_sql_calls.rs` for SELECT expressions, whole-row arguments, LATERAL/composite expansion, prepared calls, result descriptors and volatile invocation counts; reuse array owners and report the dispatch prototype's runtime in seconds (FR-172–FR-173).
- [ ] T131 [US4] Enforce routine null-input/volatility/security/settings and supported planner attributes through existing planner, auth and transaction owners, with scoped frames in `backend/crates/kalamdb-core/src/functions/host.rs` and the V8 host boundary. Add `backend/crates/kalamdb-core/tests/pg18_routine_context.rs` for STRICT/variadic NULL, volatile defaults, definer/search_path shadowing, grants, SET restoration, failed transactions, forbidden function commit and legal/illegal procedure transaction contexts; record unimplemented attribute/control cases as blocking gaps (FR-082–FR-083, FR-172).
- [ ] T132 [US3] Finalize language/body lifecycle in `backend/crates/kalamdb-handlers/crates/ddl/src/procedure/create.rs`, existing routine executors and CLI function builders: language identifiers resolve once, JS/TS compile by committed revision with bounded caches, and failed compile/publication preserves the prior executable. Extend T110 with SQL string-body versus SQL-standard body dependency/binding timing, PLpgSQL execution and unknown-language failures; keep bodyless project declarations an explicit extension, never treat SQL/PLpgSQL source as JS (FR-082, FR-173).
- [ ] T133 [US3] Extend T113/T114/T117 with typed request/reply and explicit binding forms in `backend/crates/kalamdb-functions/src/abi/`, existing host channel owners and `backend/crates/kalamdb-core/src/functions/{convert,executor}.rs`. Replace `pack_named_call_input` and `json_object_has_key` on the new ABI with cached ordinals and owner references; return typed values/errors without encode/decode roundtrips, preserve old ABI behavior behind its bounded adapter, and remove NULL-on-conversion-failure fallbacks. Test both directions and nested replies, including zero-argument and one-composite calls (FR-160, FR-165, FR-171, FR-174).
- [ ] T134 [US3] Define and implement new-ABI indexed/named input views and scalar/composite/set/procedure-output mappings in `backend/crates/kalamdb-functions/src/abi/convert.rs` and `backend/crates/kalamdb-functions-host/src/spec.rs`, extending T115/T118/T119. Directly convert new JS objects once, rewrap compatible returned native owners, and specify unnamed/quoted/prototype-like names, NULL/undefined/void, missing/extra OUT fields, cycles/getters, async mutation, cancellation and zero-row versus NULL-result behavior. Test `backend/crates/kalamdb-functions/tests/v8_regressions.rs` without an intermediate request/result serializer (FR-161–FR-165, FR-174–FR-175).
- [ ] T135 [US5] Project signature/kind/mode/default/result/language/security metadata from the same routine/type owners into `backend/crates/kalamdb-system/`, `backend/crates/kalamdb-views/` and pgwire descriptions; include `pg_proc`, `pg_type`, `pg_language`, dependency links, identity helper functions and information_schema routine views. Add `backend/crates/kalamdb-core/tests/pg18_routine_catalog.rs` for overloaded/quoted objects, atomic publication, grants, rollback/restart, replacement and stale prepared metadata; no name-only lookup or second catalog (FR-170, FR-172, FR-175).
- [ ] T136 [US8] Add the type/routine differential corpus to `benchv2/pg18/` and matrix evidence: run unchanged SQL/PLpgSQL fixtures on the pinned PG18 server and KalamDB through HTTP SQL and prepared pgwire, then paired JS bodies with equivalent public behavior. Compare acceptance/rejection, SQLSTATE, values, type/OID relationships, output names/cardinality, permissions and transaction outcomes for all `contracts/routines-postgresql.md` cases. Do not count JS body portability or syntax-only tests as runtime compatibility (FR-168–FR-173, FR-176).
- [ ] T137 [US3] Extend T121 with a reviewed migration and disposable-project journey across `cli/`, generated function/SDK bindings, stored routine definitions, schedules/triggers, grants and artifact lookup: migrate legacy PROCEDURE RETURNS and CALL users explicitly, key overloads by typed signature, and preserve quoted identifiers without JS source interpolation. Update `../KalamSite/content/sdk/**`, `../kalamdb-skills`, generated mirrors and relevant `docs/architecture/` when the behavior lands; verify build, SQL invocation, nested view forwarding, restart and rollback (FR-175).
- [ ] T138 [US8] Extend T112/T122 in `backend/crates/kalamdb-functions/benches/v8_runtime.rs` and `benchv2/pg18/` with per-direction invocation-envelope encode/decode counts, native owner identity, copied bytes, sparse/full materialization and overload/default/SQL-per-row dispatch cases. Require zero intermediate envelopes and zero payload copies on compatible forwarding, measure ordinary JS object conversion separately, and apply existing CPU/allocation/tail/retained-memory budgets to prior KalamDB and equivalent PG workloads. Record runtimes in seconds and deletion evidence for old call packing in `contracts/compatibility-matrix.md` and `contracts/cleanup-ledger.md` (FR-166, FR-174, FR-176).

**Order and checkpoint**: T125/T127 and T126 start with contract fixtures; T126 precedes T128/T129 and catalog/artifact migration. T129/T132 feed T130; T129 defines T133/T134, which share T113–T120 safety gates. T131 applies continuously. T135/T137 consume the final descriptor/ABI. Begin T136 fixtures and T138 baselines before behavior edits, and close them after integrated execution. Routine acceptance requires these tasks plus existing T110/T120/T122; unrelated G3 work can proceed independently. No zero-copy or PG-performance claim is made from this checklist alone.

## Phase 14: CLI schema diff and development lifecycle (2026-10-13)

Implement [cli-development.md](contracts/cli-development.md), preserving the everyday command contract in `specs/035-cli-everyday-lifecycle/spec.md`. Existing code has table/type/procedure diffing and lifecycle handling, but the scoped review found independent parsing/emission, stale-state risks and incomplete PG coverage. These are additional implementation obligations, not a claim that the current loop passes or that completed tasks must be unchecked.

- [ ] T139 [US7] Unify project loading in `cli/src/workflow/schema/load.rs`, `cli/crates/kalam-schema-diff/src/parser/` and `backend/crates/kalamdb-dialect/src/contracts/compile.rs` around one frozen source/AST revision consumed by diff/gen/build. Support file/directory sources, identity manifests, quoted components and resolved environment/search_path; remove the hardcoded `public`/default-env mismatch and silent unsupported-statement fallthrough. Add `cli/crates/kalam-schema-diff/tests/pg18_project_snapshot.rs` covering missing/unreadable sources, multi-file ordering and unsupported CREATE FUNCTION/DOMAIN/view forms; new models use one file each (FR-177).
- [ ] T140 [US7] Extend `cli/crates/kalam-schema-diff/src/model.rs` and `src/emitter/` to represent schema/table/column lifecycle, constraints/FKs/indexes, identity/default/nullability/collation and related object coverage from the canonical model. Emit PG ALTER clauses instead of MODIFY COLUMN, support identity-backed rename/schema moves and return typed blocked/manual operations instead of comment-only success. Add `cli/crates/kalam-schema-diff/tests/pg18_table_schema_diff.rs` with roundtrip/no-op and data-preserving change fixtures, plus explicit gap rows for unsupported forms (FR-178).
- [ ] T141 [US7] Extend existing `backend/crates/kalamdb-dialect/src/contracts/{snapshot,diff}.rs` and CLI consumers with T126/T128 type/domain/routine identities, overloads, body/language/settings/grants/comments and PG function/procedure emission. Test legal enum/composite evolution and incompatible enum/routine changes in `cli/crates/kalam-schema-diff/tests/pg18_type_routine_diff.rs`; stop automatic unsafe type recreation and unqualified drop-by-name. Keep implicit table row types owned by tables, preserve quoted SQL and distinguish inline-body DDL from external-handler-only artifact updates (FR-178).
- [ ] T142 [US7] Replace concatenated table/contract SQL in `cli/crates/kalam-schema-diff/src/diff.rs` with a deterministic dependency/risk/transaction operation graph, deriving SQL and review output from it. Replace line-based `cli/src/workflow/agent.rs` destructive detection with shared AST/operation analysis for all object families and manual migrations. Test creates/drops across types/tables/routines/defaults/triggers/FKs, dependency cycles, enum commit boundaries, supported nontransactional DDL, quoted/body DROP text, force restrictions and stale-plan rejection (FR-179).
- [ ] T143 [US7] Replace current-file baseline copying/seeding in `cli/src/workflow/dev/watch.rs` and migration preparation with exact applied snapshots and target/database/managed-scope/history receipts. Reconstruct missing baselines from verified history/catalog evidence; detect out-of-band drift, target switches, fresh clones, missing/edited migrations, branch switches and reset/restore. Add focused workflow tests proving pending/manual/skipped/generation-only work and edits during apply cannot advance the applied baseline (FR-180).
- [ ] T144 [US4] Coordinate dev/migrate/deploy through `cli/src/workflow/db/migration/apply/`, `cli/src/workflow/sql.rs` and existing backend migration/transaction owners: server-visible locking/fencing, expected-head checks, atomic DDL/receipt commits for supported groups and durable partial progress otherwise. Make `cli/src/workflow/db/migration/markers.rs` lexer-aware so `-- UP`/`-- DOWN` inside bodies cannot truncate SQL. Test concurrent clients, failure at every commit/receipt boundary, unknown outcomes, stale locks and repair without blind replay or ignored missing history (FR-181).
- [ ] T145 [US7] Refactor `cli/src/workflow/dev/{orchestrator,watch}.rs` and `cli/src/workflow/functions/{build,activate}.rs` to prepare from T139, preflight staged contracts/artifacts, apply T144 and publish against committed schema/type/ABI revisions. Extend existing backend activation owners as needed for staged artifacts, expected-active-revision checks and atomic incompatible signature/artifact handoff. Test compile/apply/activation/local-write failure separately; preserve compatible prior artifacts and resume pending stages without reapplying committed DDL or activating failed-schema outputs (FR-182).
- [ ] T146 [US7] Replace maximum-mtime acknowledgement in `cli/src/workflow/dev/watch.rs` and orchestrator state with bounded content revisions/event coalescing, distinct observed/in-flight/success/retry states and transient-failure backoff. Cover file/directory create/delete/rename/atomic saves, same-mtime edits, save-during-build/apply, configuration/identity/lockfile changes and watcher overflow; exclude generated/build/dependency/runtime outputs, bound traversal/caches and terminate owned compiler children on cancellation (FR-183).
- [ ] T147 [US3] Reuse one project descriptor in `cli/src/workflow/schema/{gen,procedure_bindings}.rs`, language generators and `cli/src/workflow/functions/build.rs` for staged generation, registry, artifact and manifest hashes. Include actual implementation/dependency/toolchain/ABI inputs; remove repeated contract compilation, fail required missing typechecking tools, skip unchanged writes and prune only manifest-owned stale outputs. Test quoted/dotted/prototype-like names, overloads, supported TypeScript/Dart/Rust declarations, ABI migration and preservation of handwritten handlers (FR-184).
- [ ] T148 [US9] Reconcile `cli/src/workflow/{target,instance}.rs`, lifecycle owners, `db/{seed,reset}.rs` and command dispatch with G2/035: one resolved target and schema semantics, ownership-aware reuse/shutdown, server-version/capability checks, safe config/env migration and receipt invalidation on database recreation. Test project/global/named environments, busy ports, stale PIDs, credential isolation, seed ordering/once-only behavior, no production auto-seed and local-only reset without broadening force into remote reset (FR-185).
- [ ] T149 [US7] Extend existing lifecycle/dev status and `cli/src/workflow/dev/{display,session,orchestrator}.rs` with correlated schema/codegen/function/app states and revisions, compatible `KALAM_READY` evolution and actionable failures. Test foreground/background/agent/JSON modes, schema paused with prior app running, missing compiler, failed activation, immediate app exit and unknown health; no ready event or successful stage label may imply unapplied/generated/unactivated work is complete (FR-186).
- [ ] T150 [US9] Make `cli/src/workflow/dev/exec.rs`, migration commands and `cli/src/workflow/deploy/` consume the same snapshot/plan/receipt rules. Isolated exec must override inherited target selectors, prepare intended schema plus required generated/activated JS routines, apply its explicit pending-draft policy, seed and preserve child exit/signal/cleanup behavior. Test deploy dry-run/no writes, fresh-clone replay, immutable checksums, capability mismatch and incompatible function rollback; update canonical CLI skills/docs and architecture records with T111/T137 when behavior lands (FR-185).
- [ ] T151 [US8] Add `cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs` for the real-server disposable journey in `contracts/cli-development.md`: init/up/dev, schema/type/domain/table/overloaded routine changes, generation/V8 activation, SQL/SDK calls, invalid edit recovery, reviewed destructive change, restart/clone/rollback/seed/reset/exec/down. Include quoted/dotted Unicode names, directories, arrays and tagged bodies; verify catalog/data/ABI behavior and subscription reconnect per T123. Rebuild CLI first and add observed-runtime async timeouts; fake-server tests alone do not satisfy this task (FR-187, SC-015).
- [ ] T152 [US8] Extend CLI workflow and backend integration tests with lifecycle fault injection: stale plan, missing baseline/history/tool, wrong target, concurrent sessions, save during each stage, lost commit response, interrupted nontransactional phase, activation/CAS/local-publication failure, stale rollback and watcher overflow. Assert no duplicate destructive effect, wrong-target write, silent missing operation, erased handwritten file, lost edit or false readiness; record recovery outcomes in `contracts/compatibility-matrix.md` (FR-180–FR-187, SC-016).
- [ ] T153 [US8] Add CLI cycle workloads to `benchv2/pg18/` for small/large/multi-file projects, cold/warm init and edit-to-ready, body-only/no-op edits, settled idle and long-running watch churn. Capture baseline before T139/T145/T146/T147, report seconds/CPU/allocations/retained bytes/parse-build-upload-write counts, apply preregistered regression budgets and verify bounded caches/processes. Close replaced parser/emitter/baseline/watch/build paths in `contracts/cleanup-ledger.md`; finalize real-project evidence before T077/T080/T081 release acceptance (FR-187, SC-016).

**Order and checkpoint**: T139 and T153 baselines start before broad CLI migration. T140/T141 consume the G2/G3/routine contracts; T142 depends on their typed operations. T143/T144 establish committed-state authority. T145/T147 share one staged revision and depend on T113/T126/T132/T134 ABI/routine contracts, with T146 developed alongside. T148–T150 preserve command parity. T151/T152 test each slice and the integrated loop; T153 closes performance/cleanup. Transaction, dependency or activation capabilities that are missing remain explicit blocking gaps, never a successful CLI stage. Preserve existing completed task descriptions and statuses.
