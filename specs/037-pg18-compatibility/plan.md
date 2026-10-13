# Implementation Plan: PostgreSQL 18 Compatibility for KalamDB 0.8

**Branch**: `feat/pg-dialect` | **Date**: 2026-10-11 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/037-pg18-compatibility/spec.md`

**Second review**: [review.md](review.md), 2026-10-11. Design corrections are incorporated below and in the contracts. Implementation and performance remain unverified.

**Routine follow-up**: 2026-10-13. [routines-postgresql.md](contracts/routines-postgresql.md) closes design gaps in type/routine syntax, quoting, overload identity and the in-memory JS call boundary. T125–T138 supplement the existing tasks; completed parser/schema tasks do not establish these cases.

**CLI lifecycle follow-up**: 2026-10-13. [cli-development.md](contracts/cli-development.md) adds complete schema/type/routine diff coverage and a coherent `kalam dev` lifecycle. T139–T153 cover source snapshots, ordered migrations, target-specific baselines, staged V8 activation, watch/recovery and real-project evidence; general SDK or smoke tasks do not close these cases.

## Summary

Make KalamDB 0.8 compatible with PostgreSQL 18 for the ordinary developer journey: one PostgreSQL grammar, schemas instead of namespaces, one logical type model, matching execution and transactions, truthful catalogs, standard clients, and a checked migration path. Full compatibility is the goal. A gap stays incompatible until an explicit product decision says otherwise.

Technical approach, from [research.md](research.md): finish the existing dialect plan in `specs/036-postgresql-sql-dialect` instead of designing a second parser. Production SQL uses `KalamDbDialect`, KalamDB's own dialect built on `PostgreSqlDialect` by delegation, not a type alias and not a direct use of the upstream dialect. Hand one parsed statement to DataFusion `statement_to_plan`. Replace `NamespaceId` with `SchemaId` through a batched rewrite program. Make `LogicalTypeRef` the only type meaning and derive Arrow and wire descriptors from it. Share catalog providers across HTTP SQL and PostgreSQL connections. Carry the same logical descriptors and immutable Arrow owners through function arguments/results and V8 read views, using [contracts/functions-v8.md](contracts/functions-v8.md) for lifetime, mutability, transfer and memory rules. Every slice deletes the code it replaces. Wide renames and test updates run as a program in bounded batches, not as a hand edit of the whole tree.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021)

**Primary Dependencies**: sqlparser 0.62, DataFusion 55.2, Arrow 59, pgwire (workspace pin, `kalamdb-postgres-wire` only), rusty_v8 152.2.0 (resolved Cargo.lock, `kalamdb-functions`)

**Storage**: RocksDB entity stores in `kalamdb-store`; Parquet segments in `kalamdb-filestore`. Hot rows use the KOBJ envelope from `kalamdb-serialization` with the schema-aware row body v2 in [contracts/row-storage.md](contracts/row-storage.md). FlexBuffers stays the cold body for catalog structs. PostgreSQL physical storage is not the target.

**Testing**: `cargo nextest run` for the crate or test target touched by a batch; CLI smoke and SDK e2e only after a running server and a rebuilt CLI. Differential checks against a pinned PostgreSQL 18 server.

**Target Platform**: KalamDB server, CLI, PostgreSQL wire clients, SDKs, and the administration UI

**Project Type**: database server with CLI, SDKs, and administration UI

**Performance Goals**: No extra SQL text rewrite on user SQL. Parse each source unit once; reuse its AST for dependency-driven reanalysis and safe pre-execution retries. Prepared parameter and result metadata share one analysis/plan revision, not one immutable plan forever. Type layouts are cached per schema or plan revision. Query latency, parse cost, allocations, and retained memory stay inside budgets recorded before the behavior change. Function forwarding reuses native owners; V8 conversions report actual copied bytes and account managed heap, external buffers and pinned Arrow data together.

**Constraints**: One dialect, `KalamDbDialect`, built on the PostgreSQL dialect. One schema identity, one logical type model, one catalog authority. Kalam extensions stay explicit and minimal. Production paths do not construct `PostgreSqlDialect` or `GenericDialect` directly. `pg_namespace` and its namespace-named columns stay. VersionId ordering (#359) and one persisted owner per shared table (#391) stay. Temporary adapters need one owner and a removal milestone. Do not edit generated SDK output by hand.

**Scale/Scope**: Nine open trackers, #392–#400. `NamespaceId` appears across backend crates, tests, CLI, SDKs, and docs. The rename and the matching test updates are too large for one editing pass.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Before research | After Phase 1 design |
| --- | --- | --- |
| I. Performance-first | Pass, if planning consumes the AST and hot paths do not gain a text rewrite or a second registry lookup per row | Pass. `statement_to_plan` is the planning boundary. Layout cache and deletion of duplicate mappings are in the type contract. Budgets are recorded before each behavior change. |
| II. Boundary ownership | Pass, if parse stays in `kalamdb-dialect`, schema and type models stay in `kalamdb-commons`, storage stays in store and filestore, and wire codecs stay in `kalamdb-postgres-wire` | Pass. Catalog meaning moves to shared providers. The wire crate consumes them. Core orchestrates and does not own a second type or schema catalog. |
| III. Minimal dependencies | Pass, if sqlparser, DataFusion, Arrow, and pgwire are reused and no second parser or type crate is added | Pass. No new runtime crate. The rewrite program is a repository script, not a server dependency. |
| IV. Validation and docs | Pass, if each gate has a command in [quickstart.md](quickstart.md) and SDK terminology changes update tests and KalamSite docs | Pass. Quickstart maps each gate to a narrow command. Generated clients are regenerated by their owning tools. |
| V. Composable APIs | Pass, if SDKs and the administration UI consume the schema and type contracts instead of keeping namespace or type mirrors | Pass. Public contracts use `SchemaId` and the logical type. UI and SDK layers do not become a second source of type or schema meaning. |

The design intends to satisfy these gates; they are not measured passes. Baselines, recovery experiments and semantic tests remain required. Versioned decoders and bounded migration adapters are the explicit temporary extra paths, never second authorities.

## Project Structure

### Documentation (this feature)

```text
specs/037-pg18-compatibility/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── compatibility-matrix.md
│   ├── schema-migration.md
│   ├── type-system.md
│   ├── functions-v8.md
│   ├── routines-postgresql.md
│   ├── cli-development.md
│   ├── execution-performance.md
│   ├── row-storage.md
│   ├── cleanup-and-rewrite.md
│   └── cleanup-ledger.md
├── spec.md
└── tasks.md                 # /speckit-tasks, not this command
```

Dialect contracts that G1 must reconcile, not copy:

```text
specs/036-postgresql-sql-dialect/
├── plan.md
├── contracts/parser-architecture.md
├── contracts/sql-language.md
├── contracts/capability-matrix.md
└── contracts/efficiency-security-cleanup.md
```

### Source Code (repository root)

```text
backend/crates/kalamdb-dialect/          # one PostgreSQL parse and classification
backend/crates/kalamdb-commons/          # SchemaId, LogicalTypeRef, column and table models
backend/crates/kalamdb-core/             # SQL orchestration, statement_to_plan, TypeRegistry
backend/crates/kalamdb-system/           # authoritative schema catalog, today under providers/namespaces
backend/crates/kalamdb-views/            # catalog projections shared by both SQL entry points
backend/crates/kalamdb-postgres-wire/    # startup, extended query, codecs; not a second planner or catalog
backend/crates/kalamdb-functions/        # native value transport, V8 adapters, invocation budgets
backend/crates/kalamdb-functions-host/   # generated ctx APIs from one host contract
backend/crates/kalamdb-core/src/functions/ # binding, host SQL and routine orchestration
backend/crates/kalamdb-store/            # entity keys; no SQL parsing
backend/crates/kalamdb-serialization/    # KOBJ envelope, row body v2, RowView, payload-to-RecordBatch
backend/crates/kalamdb-tables/           # row codecs and DML write path from Arrow arrays
backend/crates/kalamdb-flush/            # flush consumes the shared payload-to-RecordBatch function
backend/crates/kalamdb-filestore/        # Parquet lifecycle; no SQL parsing
backend/crates/kalamdb-handlers/         # DDL execution, including today's namespace handlers
cli/                                     # schema terminology and fixtures
link/sdks/                               # generated contracts plus handwritten SQL producers
ui/                                      # administration UI copy and SQL studio
pg/                                      # extension bridge; emits canonical SQL, not a second Kalam parser
scripts/pg18-rewrite/                    # batched rename and fixture program
docs/pages/kalamdb-0.8-epics.md          # living roadmap
```

**Structure Decision**: Stay inside the current workspace. G1 follows the 036 module split. G2 renames the namespace provider and handlers in place. G3 extends `LogicalTypeRef` and `TypeRegistry` instead of adding a type crate. Catalog providers live with system metadata and views. The PostgreSQL wire crate keeps connection and encoding only.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
| --- | --- | --- |
| One temporary decoder for already-stored namespace fields or messages | A symbol rename must not corrupt existing data or mixed-version messages | Rewriting every stored key in the same change as the type rename can destroy data if the key format embeds the old name. The decoder sits at one boundary, has an owner, and has a removal milestone. It is not a second schema catalog. |
| The v1 row body decoder stays beside the v2 codec | Rows already on disk must decode until update, flush, or compaction rewrites them | A stop-the-world rewrite of every RocksDB value on upgrade blocks the release journey and risks data on interruption. Both bodies live in `kalamdb-serialization` behind one envelope flag, and v1 has its own removal milestone. |

## Delivery

Dependencies are gates, not whole-epic closure. Verification baselines start with G1. Catalog and protocol work overlap after their start gates. P2 release work is required, later.

### Phase A — Baseline (#397 starts)

Record pinned PostgreSQL 18 and client versions, the matrix skeleton in [contracts/compatibility-matrix.md](contracts/compatibility-matrix.md), current failures, and latency and allocation baselines before behavior changes. Seed [contracts/cleanup-ledger.md](contracts/cleanup-ledger.md) from the inspection below and refresh it against the revision being changed.

### Phase B — G1, one grammar and one plan (#392)

Reconcile `specs/036-postgresql-sql-dialect` with this revision, then finish that plan. Do not open a second parser design.

`KalamParser` is configured with `KalamDbDialect`. Replace the current alias in `kalamdb-dialect/src/dialect.rs`:

```rust
pub type KalamDbDialect = sqlparser::dialect::PostgreSqlDialect;
```

with KalamDB's own dialect struct that delegates to `PostgreSqlDialect`. Use the pinned sqlparser `derive_dialect!` with `preserve_type_id = true`, as specified in tasks.md, after T002 verifies the resolved dependency API. Avoid a handwritten partial delegation. Reverify identity and inherited behavior on upgrades. Kalam-only statement forms are recognized by that dialect and by `KalamParser` on the same cursor. They must not capture a statement PostgreSQL already defines. Unregistered permissive syntax stays off.

DataFusion's session dialect stays `postgresql` (`SQL_PARSER_DIALECT` in `kalamdb-core/src/sql/datafusion_session.rs`) only as the defensive default for internal planning. User SQL is parsed by `KalamDbDialect` before planning. Workspace pins are DataFusion 55.2.0, Arrow >=59.2.0, sqlparser >=0.62.0. The 036 plan still says DataFusion 55.1.0; use 55.2.

Production paths that still parse with `GenericDialect` or plan SQL text, and that G1 must move or delete:

- `kalamdb-dialect/src/classifier/engine/core.rs`
- `kalamdb-dialect/src/ddl/create_view.rs`, `alter_table.rs`, `user_commands.rs`
- `kalamdb-core/src/sql/executor/sql_executor/mod.rs` (`GenericDialect` and `session.sql`, including retries)
- `kalamdb-postgres-wire/src/statement.rs` (`create_logical_plan` separately for parameters and results)
- `kalamdb-row-filter/src/parser.rs`

Test-only `GenericDialect` uses stay only as explicit rejection or comparison tests.

First slice: one `SELECT` and one parameterized DML through HTTP SQL and the wire, including prepared metadata and a retry, using `SessionState::statement_to_plan` on the shared statement. Then move remaining callers. Attach G1 evidence before broad schema or type migration.

The parser corpus also runs against PostgreSQL 18, not only sqlparser. Replanning is allowed; reparsing the same source unit is not. Cache keys, byte admission, session lifetimes and retry boundaries follow [contracts/execution-performance.md](contracts/execution-performance.md).

### Phase C — G2, schemas (#400)

Write the persisted-format and public-interface decision before renaming stored keys. Then run the rewrite program in [contracts/cleanup-and-rewrite.md](contracts/cleanup-and-rewrite.md), one crate or one test directory per batch.

First separate stable schema identity from a quoted-aware display name: the current `NamespaceId` is a lowercased name and current `TableId` embeds it. T099 proves stable keys, OIDs, dependencies and restart after rename/move before the broad symbol migration. Then replace `NamespaceId` (`kalamdb-commons/src/models/ids/namespace_id.rs`) with `SchemaId` and migrate consumers. Move `kalamdb-system/src/providers/namespaces/` and `kalamdb-handlers/crates/ddl/src/namespace/` to schema names in the same batches that delete the old modules. Keep `pg_namespace`, `nspname`, `nspowner`, `nspacl`, `typnamespace`, `pronamespace`, and `relnamespace`.

Gate slice before downstream migration: `CREATE SCHEMA`, ordinary `CREATE TABLE`, lookup, restart.

### Phase D — G3, one type model (#393)

Design review before broad migration, using [contracts/type-system.md](contracts/type-system.md). Prove `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array through planning, hot storage, Parquet, prepared descriptions, and text and binary results.

`ColumnDefinition` in `kalamdb-commons/src/models/schemas/column_definition.rs` still stores `data_type: KalamDataType`, `named_type_id`, and `is_array` beside `LogicalTypeRef`'s existing shape. Those parallel fields are deletion targets once consumers read one logical type. Do not add a new PostgreSQL type enum next to `KalamDataType`.

`TypeRegistry` stays in `kalamdb-core/src/schema_registry/type_registry/`. Wire `statement.rs` and row encoding stop keeping their own type tables. Unknown types are not advertised as text.

Row storage is a separately gated slice within G3, per [contracts/row-storage.md](contracts/row-storage.md). Type compatibility does not justify an unmeasured format rewrite. Order:

1. Measure baseline costs and fix semantic representations (including enums, arrays and historical defaults). Resolve the format compatibility barrier and downgrade policy before enabling new writers.
2. Persist immutable historical layouts and custom-type revisions, with bounded RAM caches, snapshot-aware retention and revision-exhaustion behavior. Validate transactional DDL conversion before publication; no blanket casts on read.
3. Prototype the validated v2 codec and direct Arrow materialization. Compare packed/aligned and view/compact-buffer alternatives using representative workloads.
4. Migrate DML and flush without losing constraints, atomicity, versions, tombstones or ownership lifetimes. Delete replaced materialization only when all consumers pass.
5. Enable v2 writes only after correctness, mixed-format recovery and quantitative time/allocation/memory gates pass. Independent parser/cache/copy improvements can ship before this switch.

Primitive arrays and encoding still copy; borrowed variable-width views are an optimization with measured retention limits. No new serialization crate. FlexBuffers remains for catalog structs. Out-of-row storage is not silently bundled into v2.

### Phase D2 — Function values and V8, aligned with G3

The user-requested extension is [contracts/functions-v8.md](contracts/functions-v8.md), T112–T122. Baseline the current call path alongside T096; agree logical-type/ABI mappings alongside T044/T100, before changing RoutineValue or generated signatures. Keep function transport independent of the v2 disk activation gate.

Reuse native Arrow cell/batch owners instead of rebuilding per-row scalar arrays. Materialize existing mutable JS results directly once; offer explicit read-only views for no-copy query→function→nested-call forwarding. Preserve child realms and permission frames. Binary transfer requires exclusive ownership or explicit guest detachment; immutable database buffers are not exposed as mutable typed arrays. Text/numeric/temporal conversion remains lossless and measured.

Bound native, managed and external memory together, including async queues, results and retained views. T116/T120 lifecycle checks precede enabling fast paths; T118 covers type fidelity, T119 stream backpressure, T121 generated TypeScript/docs, and T122 measured CPU/allocation/copy/memory gates. Extend existing runtime and cache owners; do not add a second serializer/type registry or route SQL/plpgsql through V8.

### Phase D3 — PostgreSQL routine surface and typed calls

Implement [routines-postgresql.md](contracts/routines-postgresql.md) through the shared AST and existing type/routine owners. JS/TS functions use ordinary PostgreSQL function signatures and SELECT/expression calls; JS/TS procedures use CALL and output parameters. Their declared language selects V8. SQL/PLpgSQL semantics remain independently required by T110. No SDK-specific invocation wrapper is required from SQL.

T125/T127 extend lexer/type tests to the actual classifier and handlers, including quoted names, tagged bodies, empty/Unicode enum labels and table row types. T126 defines stable routine identity and overload keys before T128 migrates DDL/catalog consumers. Correct pending T043 to discard function typmods. T129 binds defaults, modes, named/positional arguments and composite values explicitly. T130 proves DataFusion expression/set/table invocation with bounded async V8 execution; T131/T132 enforce attributes, transactions and language lifecycle. No return-value cache may change volatile behavior.

T133/T134 refine T113–T119 with explicit binding forms and typed request/reply messages. Remove JSON packing and shape heuristics from the new ABI in both directions. Compatible Arrow views reuse owners; ordinary guest objects convert directly once. T135/T137 migrate catalog/artifact/SDK consumers, preserving identity, quoting and existing grants. T136 supplies PG differential and paired JS fixtures; T138 supplies zero-intermediate-envelope and copy-count evidence alongside T122. These tasks gate routine acceptance even where earlier parser tasks are checked off; they do not depend on enabling the new disk format.

### Phase E — G4, execution and transactions (#394)

Implement against completed type families. Differential fixtures compare results, types, SQLSTATE, permissions, and commit or rollback with PostgreSQL 18. Do not weaken VersionId or shared-table shard ownership. Record the cross-group transaction boundary as a gap owned with #359 and #391, not as a compatibility pass. G4 requires the full failure/concurrency matrix in [contracts/execution-performance.md](contracts/execution-performance.md), including savepoints, snapshot rules, atomic writes, DDL, retry ambiguity and live-event visibility. One rollback and one concurrent write are preliminary smoke checks only.

### Phase F — G5 catalogs and clients (#399, #395)

After G2 and G3 contracts exist, implement catalog providers from authoritative records and delete `kalamdb-postgres-wire/src/client_catalog/empty_tables.rs` entries as real rows replace them. Both SQL entry points call the same providers.

Protocol work starts after G1 and G3 and consumes schema and catalog interfaces as they land. Reuse the #354 wire foundation. Prepared describe, bind, and encode share the G3 descriptors. Final client acceptance includes catalog integration and relevant G4 behavior. #375 stays the identity-override issue.

### Phase G — Tooling (#396)

After ordinary-workflow G4, G5, and the required client slice: capture real introspection and migration statements, run them against the shared catalogs, and validate a PostgreSQL-to-KalamDB schema, data, sequence, constraint, and index path. #399 owns catalogs. Extensions, procedural languages, backup, and replication stay separate matrix rows.

Migration compares typed contents, constraints, indexes and sequence state, not just row counts. Session and protocol state-machine cases are mandatory before client acceptance.

### Phase H — Final verification and release (#397, #398)

Full regression, deletion review against the cleanup ledger, performance budgets, function-value and V8 evidence for SC-012 and SC-013, and the unchanged ordinary PostgreSQL journey on the release candidate. Release artifacts then pass install, upgrade, backup, restore, restart, and interrupted recovery. Release text matches the matrix.

### CLI development slice — starts with G1/G3, closes before Phase H

Follow [cli-development.md](contracts/cli-development.md), preserving the 035 everyday command and process-ownership contract. T139 loads one frozen project revision through the shared AST/resolver, including file/directory sources and the resolved environment. T140/T141 cover schema/table/type/function/procedure diffs; T142 orders one typed operation graph and classifies risk instead of concatenating independent emitters or inspecting SQL lines. Every unsupported change blocks explicitly rather than disappearing into a comment/no-op.

T143 makes baselines target- and receipt-specific, with drift/fresh-clone reconstruction. T144 provides concurrency-safe migration application, transactional receipts and explicit nontransactional recovery. T145 stages compilation and generation before effects and binds activation to the committed schema/ABI revision; no filesystem/database atomicity is assumed. T146 retains edits that arrive during work and retries transient failures without output-triggered rebuild loops. T147 publishes generated artifacts from that same snapshot.

T148–T150 reconcile target resolution, up/down/reset/seed, stage-specific readiness, isolated exec and migration/deploy parity. T151/T152 require real-server disposable projects and failure/concurrency injection; existing fake-server tests cannot close these gates. T153 starts baseline measurement before edits and closes idle CPU, edit-to-ready, allocation/retention and cleanup evidence. Backend transaction/catalog/ABI capabilities are prerequisites for dependent stages, not permission to declare a CLI stage successful early. Release sign-off includes SC-015/SC-016.

## Cross-cutting rules

These apply to every phase. Details are in [contracts/cleanup-and-rewrite.md](contracts/cleanup-and-rewrite.md).

1. A slice is done only after its cleanup step. The old path, helper, and mapping are deleted in that change. A new layer beside the old one fails review.
2. Mechanical namespace, schema, and repeated SQL-text updates run through `scripts/pg18-rewrite` on one batch. The batch is re-runnable. The rest of the tree is not loaded into one editing session.
3. Each batch names its exception list, the command that verified it, and the ledger rows it closed.
4. Prefer a smaller owner for existing behavior over a new module. New files are listed in the slice note before they are added.
5. SDK changes update corresponding `../KalamSite/content/sdk/**`; user-facing syntax, flags, configuration and runbooks update canonical `../kalamdb-skills` content and generated mirrors. Release versions add their release body and README link.
6. Generated files under SDK `generated` directories are regenerated by the owning `build.sh` or generator, never hand-edited.

## Validation

Use [quickstart.md](quickstart.md). A closed issue or a green parser test is not gate evidence. Evidence names the revision and the PostgreSQL and client versions.
