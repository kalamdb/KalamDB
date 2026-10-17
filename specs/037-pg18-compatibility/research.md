# Research: PostgreSQL 18 Compatibility for KalamDB 0.8

Decisions for [plan.md](plan.md). The 10 October 2026 roadmap inspection was rechecked against this workspace on 11 October 2026. It is a code reading, not a passing test run.

## Decision: Finish the 036 dialect plan instead of a second parser

**Decision**: G1 reconciles and implements `specs/036-postgresql-sql-dialect`. Production parsing uses `KalamDbDialect` in `kalamdb-dialect`. That type is KalamDB's own dialect, built on `PostgreSqlDialect` by delegation: PostgreSQL identity, lexical rules, operator precedence, quoting, and statement parsing come from the PostgreSQL dialect, and only documented Kalam extensions are added on top. It is not a type alias of `PostgreSqlDialect`, and it is not a second grammar. Classification, authorization, prepared metadata, and execution share one parsed statement. DataFusion plans that statement with `SessionState::statement_to_plan`. The DataFusion session setting `datafusion.sql_parser.dialect = postgresql` stays a defensive default for internal planning. User SQL does not enter through that setting or through `PostgreSqlDialect` directly.

**Rationale**: `dialect.rs` is currently `pub type KalamDbDialect = sqlparser::dialect::PostgreSqlDialect`. A type alias cannot carry Kalam extensions, and callers can still construct `PostgreSqlDialect` or `GenericDialect` beside it. sqlparser 0.62 does not inherit `Dialect` implementations through a containing struct, so the Kalam dialect must delegate explicitly, including `Dialect::dialect()` identity, as `specs/036-postgresql-sql-dialect/research.md` R-002 requires. The 036 parser contract already configures `KalamParser` with `KalamDbDialect`. Keeping the alias would leave the upstream dialect as the real implementation.

**Alternatives considered**: Leaving the type alias; constructing `PostgreSqlDialect` at each call site; a new parser crate; keeping `GenericDialect` for Kalam commands. Rejected. The alias cannot host extensions. Direct upstream use bypasses Kalam's dialect. A new parser crate is a second grammar. `GenericDialect` is the legacy path G1 deletes. Confirmed remaining production call sites include `classifier/engine/core.rs`, `ddl/create_view.rs`, `ddl/alter_table.rs`, `ddl/user_commands.rs`, `sql_executor/mod.rs`, `postgres-wire/src/statement.rs`, and `kalamdb-row-filter/src/parser.rs`.

## Decision: DataFusion 55.2 statement_to_plan is the planning boundary

**Decision**: Use the workspace pin DataFusion 55.2.0 and Arrow >=59.2.0. One plan supplies prepared parameter metadata and result metadata. Reanalysis/replanning consumes the retained AST. Nested ASTs are reused; dynamic SQL and string language bodies are separately counted source units. Descriptions share one plan revision; plans are not assumed valid forever.

**Rationale**: `statement.rs` currently calls `create_logical_plan` twice on SQL text, once for parameters and once for results. `sql_executor` calls `session.sql` on the execution string, including retry sessions. That is two plans and a text reparse. `statement_to_plan` accepts the parsed statement on the installed 55.2 API described by the roadmap inspection.

**Alternatives considered**: Staying on the 036 document's DataFusion 55.1 pin; planning parameters and results separately; a regex rewrite before DataFusion. Rejected. The workspace is already 55.2. Separate plans drift. Text rewrites violate constitution principle I and spec FR-021.

## Decision: Do not treat DataFusion extension metadata as PostgreSQL semantics

**Decision**: G3 reads the pinned DataFusion extension-type API before choosing it. Custom meaning that Arrow field metadata cannot enforce lives in the logical type catalog. Proven cast or operator gaps use a typed coercion or planning hook.

**Rationale**: The installed DFExtensionType notes describe pretty-printing as the customizable surface. Metadata propagation does not prove PostgreSQL casts, modifiers, or enum order.

**Alternatives considered**: Registering an extension type and assuming operators and casts follow. Rejected until a test shows the pinned API actually does that.

## Decision: SchemaId replaces NamespaceId by a batched program

**Decision**: Add `scripts/pg18-rewrite` and apply `NamespaceId` to `SchemaId`, plus the agreed field and module renames, one crate or one test directory per run. SQL fixtures get a separate statement rewrite with an exception file. PostgreSQL identifiers `pg_namespace`, `nspname`, `nspowner`, `nspacl`, `typnamespace`, `pronamespace`, and `relnamespace` are denied. After each batch, check only that crate, then record the batch in the cleanup ledger.

**Rationale**: `NamespaceId` is referenced across the backend, tests, CLI, bridge, and docs. `namespace_id.rs` alone has on the order of ninety mentions, and the type is used from stores, Raft commands, handlers, and SDKs. Editing that corpus in one session will miss exceptions and leave duplicates. A repeatable program keeps the exception list honest.

**Alternatives considered**: Blind search-and-replace across the repo; an agent rewriting every file in one pass; a permanent `type SchemaId = NamespaceId` alias. Rejected. A blind replace renames PostgreSQL catalog columns. A single pass is too large to review. A permanent alias is a second name for the same concept, which spec FR-035 forbids.

## Decision: Persisted format is decided before stored keys change

**Decision**: The first G2 batch inventories stored keys, Raft commands, protocol messages, configuration, and client contracts that embed namespace field names. The batch writes one decision: keep the stored bytes and translate at a single boundary, or version the format with an explicit upgrade. The translator, if needed, has one owner and a removal milestone. Only then do later batches rename in-memory types.

**Rationale**: Renaming a Rust field that is also a serde or key component can corrupt existing data or break mixed-version messages. Spec FR-037 requires that decision first.

**Alternatives considered**: Renaming serde fields in the same patch as the type name with no inventory. Rejected because it can rewrite stored identity by accident.

## Decision: One logical type, built from LogicalTypeRef, not a new enum

**Decision**: `LogicalTypeRef` becomes the type stored for columns, composite fields, parameters, routine signatures, and results. Modifiers live on that contract. `ColumnDefinition`'s `data_type`, `named_type_id`, and `is_array` are removed after each consumer moves. `KalamDataType` may remain only as the builtin payload inside that one model while it is still the compact stored tag; it must not be a second source of meaning. Arrow layouts and pgwire OIDs are derived. `TypeRegistry` remains the registry in `kalamdb-core`.

**Rationale**: `LogicalTypeRef` already distinguishes builtin, named, and list types, but it still wraps `KalamDataType`, and `ColumnDefinition` still stores the three parallel fields. Adding another PostgreSQL enum would make three sources. The spec forbids that.

**Alternatives considered**:

- Use `postgres_types::Type` as the only model. It matches wire OIDs and not Kalam file and embedding extensions, stable field slots, or storage tags.
- Use Arrow types as the only model. Arrow cannot distinguish every PostgreSQL modifier, enum order, or JSON versus JSONB.
- Keep `KalamDataType` plus `named_type_id` plus `is_array` and add PostgreSQL names beside them. That is the current split and the deletion target.

The design review records this comparison before broad migration, as spec FR-055 requires.

## Decision: Numeric keeps a fast path and an explicit wide-value decision

**Decision**: Values that fit a fixed-width decimal keep that fast path. `numeric(12,2)` is part of the vertical slice. Unconstrained or wider numeric needs a written design or a recorded scope exclusion before the matrix marks numeric verified. Renaming `DECIMAL` to `NUMERIC` is not the decision.

**Rationale**: PostgreSQL numeric precision exceeds Decimal128. Pretending a rename is semantic compatibility fails the type gate.

**Alternatives considered**: Immediate storage rewrite of every decimal; claiming numeric compatibility from the current decimal type. Rejected until the vertical slice and the wide-value decision exist.

## Decision: Catalogs are shared providers, and empty probes are deleted as they are replaced

**Decision**: Relational catalog providers live in one shared module used by HTTP SQL and the wire. Wire startup and codecs stay in `kalamdb-postgres-wire`. `client_catalog/empty_tables.rs` is an inventory of stubs, not an implementation. Each stub is removed when the authoritative provider supplies real rows. Empty is valid only when the caller truly has no visible rows.

**Rationale**: The wire crate currently publishes empty probe schemas, including gaps such as enum, dependency, cast, and language catalogs called out in #399. A wire-owned mirror would be a second catalog.

**Alternatives considered**: Filling empty tables inside the wire crate; leaving probes so clients do not error on missing relations. Rejected. Probes that return the wrong column types or hide missing objects fail spec FR-100 and FR-102.

## Decision: Protocol work extends #354 and does not retake it

**Decision**: Audit the existing wire implementation and close gaps: shared type descriptors, cancellation, COPY where the matrix requires it, SQLSTATE, and pinned clients. Do not rebuild the connection path. #375 remains a separate security fix.

**Rationale**: #354 is closed and explicitly excluded full compatibility. Reimplementing it delays the remaining gaps and risks a second codec table.

**Alternatives considered**: A new wire stack. Rejected.

## Decision: Verification starts before behavior changes and never weakens tests

**Decision**: Phase A records baselines and the matrix. Each later batch adds differential coverage and runs the migrated tests for that batch. Assertions are rewritten only when the expected behavior is preserved and the change is noted. Skips need a reason. Final stage 7 runs the full sweep on a fresh server and a rebuilt CLI.

**Rationale**: Spec FR-124 and FR-131. A green suite obtained by deleting assertions is not compatibility evidence.

**Alternatives considered**: Converting tests at the end. Rejected because the corpus and the code would drift for the whole program.

## Decision: Cleanup is part of the slice, and the better structure replaces the old one

**Decision**: Each slice's definition of done includes deleting the superseded path and updating the cleanup ledger. If two modules do the same work, one remains. New files are justified in the slice note. Temporary adapters are listed with an owner and a removal milestone.

**Rationale**: Spec FR-147 through FR-151, and constitution principles I and II. Stacking schema names on namespace modules, or pgwire mappings on `KalamDataType` mappings, leaves the duplicate the spec forbids.

**Alternatives considered**: Rename first and clean up in a later epic. Rejected. Later cleanup does not happen reliably, and the duplicate becomes the second authority.

## Second review: changes to the implementation decisions

See [review.md](review.md) for concrete flaws and evidence at `be5efdd2`. The original inspection did not measure codec speed or allocation counts. The reviewed contracts distinguish code evidence from runtime evidence.

- Mechanical renaming follows stable identity/name separation, not the reverse. Existing NamespaceId lowercases names; existing TableId includes that name.
- v2 is a separately gated physical optimization, with conditional activation; it is not automatically required by a PostgreSQL type spelling change. Historical DDL semantics and a new-reader/old-writer upgrade stage come first.
- Borrowed views reduce copies but can amplify retained memory; compare them with compacted output after selective operations.
- Existing plan-cache ownership remains; add dependency correctness and byte admission rather than a second cache. Prepared metadata reuses analysis while safe invalidation remains possible.
- An ordinary execution smoke test cannot close G4. Explicit snapshot, failure, savepoint, retry and protocol state tests are required.
- Pin the resolved Cargo.lock versions, server/client versions and benchmark configuration. Manifest lower bounds alone are not reproducible pins. Recheck the derive macro/API at T002; no dependency-cache scan or runtime experiment was performed by this review.

## Decision: Align function/V8 transport with G3 owners

User extension, 2026-10-11: reuse immutable Arrow owners for function arguments, query objects and nested results; explicit V8 read views avoid eager whole-object conversion. Preserve normal mutable objects through direct materialization, and version read-view/transfer semantics. Use existing runtime/host/core owners, not a new serializer or logical type hierarchy. See [functions-v8.md](contracts/functions-v8.md) for current-code evidence and source links.

Inspection found Arrow→scalar→single-element-array reconstruction, binary copies in both directions, nested parameter clones, and a named-input JSON packing path. The ordinary typed converter already avoids JSON, and encoded `RoutineValue.transfer` is not consumed by V8. Focus removal on measured paths rather than claiming every call serializes through JSON. Existing microsecond→millisecond lowering and permissive numeric conversion must be reconciled with G3 before a fast ABI is enabled.

Cargo.lock resolves rusty_v8 152.2.0; fetched latest API docs also identify 152.2.0. Owned backing stores and detachment provide conditional transfer mechanisms; actual sandbox/allocator features still require T112/T116 verification. V8-owned mutable buffers must not alias immutable Arrow storage. External strings are optional for compatible encodings, not a universal UTF-8 sharing mechanism. No runtime or allocation benchmark was run during this planning extension.

## T002: Resolved lock and API check

Checked on 2026-10-11 against `Cargo.lock` and the crates it resolves. The lock matches the workspace constraints. There is no version drift to correct.

| Crate | Manifest | Cargo.lock |
| --- | --- | --- |
| datafusion, datafusion-common, datafusion-datasource, datafusion-expr | 55.2.0 | 55.2.0 |
| sqlparser | >=0.62.0 | 0.62.0 |
| arrow, arrow-schema | arrow-schema >=59.2.0 | 59.2.0 |

The APIs the plan names are present on those resolved crates:

- sqlparser 0.62.0 feature `derive-dialect` exports `derive_dialect!`, including `preserve_type_id = true`.
- DataFusion 55.2.0 `SessionState::statement_to_plan` takes one `Statement` and returns a logical plan.
- arrow-array 59.2.0 `GenericByteViewBuilder` has `append_block` and `try_append_view`.

## Routine surface review — 2026-10-13

Current source based on `9cf03307` still has bespoke type/procedure parsing, lowercased name-only routine identities and JSON named-input packing. The PG18 documentation confirms the needed split between function returns and procedure output arguments, signature identity, quoted names and context-specific typmods. Full evidence and sources are in [routines-postgresql.md](contracts/routines-postgresql.md) and [review.md](review.md).

Decision: preserve PostgreSQL public DDL/call semantics while selecting V8 for declared JS/TS languages. Bind once into typed slots; forward native owners and typed replies without invocation-envelope serialization. New JS objects convert directly once, and ordinary wire/storage codecs remain. This supplements the existing V8 design instead of adding a serializer, parser or registry. T125–T138 cover the remaining implementation and verification; this review ran no runtime or performance experiment.

## CLI schema diff and lifecycle review — 2026-10-13

Current code based on `9cf03307` has schema diffing and dev lifecycle orchestration, but uses structural and contract parsers/emitters independently. Types/routines are appended after table SQL, some PG declarations fall through unhandled, generated modifications retain non-PG syntax, and constraints can become advisory comments. Baselines, watch stamps, artifact generation and activation are not all bound to one captured revision. Source evidence is recorded in [review.md](review.md).

Decision: extend existing owners with one project snapshot, a typed dependency/risk operation graph and explicit committed-state receipts. Preflight generation/build before effects; publish executables against the committed schema/ABI and expose partial-stage recovery. Preserve 035 command ownership while making file/directory watching, isolated exec and fresh-clone replay use the same contracts. The PG dependency/enum/index references in [cli-development.md](contracts/cli-development.md) determine legal ordering and transaction phases. T139–T153 add implementation and real-server/performance gates; no runtime experiment was run here.
