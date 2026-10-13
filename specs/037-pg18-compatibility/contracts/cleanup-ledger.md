# Cleanup Ledger

Seed from the 11 October 2026 reading. Refreshed after User Story 1 on 2026-10-12. `KalamDbDialect` is no longer an alias. `sqlparser_derive` 0.5.0 reads `src/dialect/mod.rs` from the invoking crate, so the derive lives in `src/kalam_dialect.rs` (`#[path]` module `dialect`) and `build.rs` writes a gitignored snapshot of the locked sqlparser `Dialect` trait at `src/dialect/mod.rs`. Production `PostgreSqlDialect {}` and `GenericDialect {}` constructions in the rows below are deleted. Test-only comparison and rejection parsers stay. Schema, type, row-storage, function, and active-work cache rows stay open.

| Old path | Replacement | Status | Notes |
| --- | --- | --- | --- |
| `pub type KalamDbDialect = PostgreSqlDialect` in `kalamdb-dialect/src/dialect.rs` | `derive_dialect!(KalamDbDialect, PostgreSqlDialect, preserve_type_id = true)` in `src/kalam_dialect.rs` | done | T011. `dialect.is::<PostgreSqlDialect>()` stays true. `Dialect::parse_statement` stays inherited. |
| Direct `PostgreSqlDialect` or `GenericDialect` construction on production paths | `KalamDbDialect` through `KalamParser` | done | T009. Test-only comparison and rejection cases stay on the exception list |
| `GenericDialect` in `classifier/engine/core.rs` | Shared parse with `KalamDbDialect` | done | T013. `CREATE USER TABLE` is rejected. Ordinary procedures and triggers are not captured |
| `GenericDialect` in `ddl/create_view.rs`, `alter_table.rs`, `user_commands.rs` | Shared PostgreSQL parse | done | T014–T016. Comments, schedules, policies, subscribe, and table DDL moved in the same pass |
| `GenericDialect` in `sql_executor/mod.rs` | `statement_to_plan` on the shared statement | done | T025 |
| `session.sql` including retry sessions in `sql_executor/mod.rs` | Replan the same statement | done | T025. Retries reuse the parsed AST. Handler `session.sql` for `CREATE VIEW` and `SET` is outside this slice |
| Two `create_logical_plan` calls in `postgres-wire/src/statement.rs` | One plan for parameters and results | done | T026 |
| `GenericDialect` in `kalamdb-row-filter/src/parser.rs` | `KalamParser` on `SELECT 1 WHERE {clause}` | done | T024. The wrapper stays a distinct expression source |
| `NamespaceId` and `providers/namespaces/` | `SchemaId` and one schema catalog | open | T032 left `pub type NamespaceId = SchemaId`. T033 moved the catalog to `providers/schemas/`. The stored table name `namespaces` and `NamespacesTableProvider` stay. DDL handlers moved to `ddl/src/schema/` (T034). Deny-list search on 2026-10-12: `pg_namespace`, `nspname`, `nspowner`, `nspacl`, `typnamespace`, `pronamespace`, and `relnamespace` still appear in `kalamdb-views` `pg_catalog` projections and catalog tests. |
| `handlers/.../namespace/` | Schema DDL handlers | done | T034. Handlers live in `ddl/src/schema/`. `CREATE SCHEMA` uses that create path. Raft `CreateNamespace` bytes stay |
| `ColumnDefinition.data_type`, `named_type_id`, `is_array` | One `LogicalTypeRef` | open | Delete after consumers move |
| Independent wire type maps and silent text fallback | Derived codecs from the logical type | open | #395 and #393 |
| `client_catalog/empty_tables.rs` stubs | Shared catalog providers | open | Delete stub as each real provider lands |
| `CREATE NAMESPACE` as a public command | `CREATE SCHEMA`, with a migration error if the old form is rejected | open | Classifier rejects the public form (T029). `CreateNamespaceStatement::parse` and `contracts/compile.rs` still read historical text |
| Per-value type tags and the u32 offset table in `row/encode.rs` | Row body v2 per `row-storage.md` | open | T090. v1 decoder stays on its own milestone |
| `ScalarValue` materialization in `row/batch.rs::decode_payloads_to_arrow_batch` and `Reader::string()` copies on the batch path | `RowView` into Arrow builders and view blocks | open | T091 |
| `record_batches_to_rows` and map-lookup encoding in `kalamdb-tables/src/utils/datafusion_dml.rs` | Encode from Arrow arrays into a pre-sized buffer | open | T093 |
| `rows_into_arrow_batch` from `Row` in `kalamdb-flush/src/flush/scope_writer.rs` | Shared payload-to-`RecordBatch` function | open | T094 |
| `Vec<u8>` per row from the RocksDB scan in `backends/rocksdb/backend.rs` | One owned block per scan chunk | open | T092 |

Test-only `GenericDialect` fixtures stay when they are explicit rejection or comparison tests. They are not rows to delete blindly.

## Review-added cleanup obligations

| Old path | Replacement | Status | Notes |
| --- | --- | --- | --- |
| Name-derived schema identity and unconditional identifier lowercasing | Stable identity plus quoted-aware names | open | T099 model is `schema_identity`. Persisted `SchemaId` and `TableId` bytes are unchanged |
| Entry-only cache admission | Byte and entry limits plus active-work admission | open | T103; retain one cache owner |
| Current-default/lazy-cast schema evolution, if found in implementation | Historical missing values and transactional DDL | open | T101; audit before deleting, this was a plan flaw, not a proven current-code defect |
| Duplicated metadata planning | One analyzed revision with dependency invalidation | open | Wire parameters and results share one plan (T026). Dependency invalidation remains with later cache work |

Temporary v1 decoders and integration adapters close only at their named compatibility/removal gates. Cleanup must not remove the only reader of supported backups or mixed-format rows.

## Function/V8 cleanup obligations

| Old path | Replacement | Status | Notes |
| --- | --- | --- | --- |
| `scalar_struct_from_row` cell→scalar→one-element-array reconstruction and list rebuilds | Native Arrow cell/batch owners; direct legacy-JS materialization | open | T114; retain proven small ScalarRows shortcut |
| Named-input JSON packing and repeated key probes in `core/functions/executor.rs` | Descriptor-driven named ordinals and typed payload | open | T114; never substitute failed conversion with NULL |
| `params.clone()` and full conversion on compatible nested calls | Move/reuse owned typed arguments and realm-local read wrappers | open | T117; preserve security-frame boundaries |
| Unconditional bytea copies on eligible exclusive transfers | Explicit ownership transfer with charged backing store | open | T116; shared/mutable/allocator fallbacks remain required |
| `RoutineValue.transfer` plus duplicated value representation when unused | One canonical descriptor and active payload | open | T114/T122 audit actual consumers before removal; durable/external formats are separate |
| Millisecond timestamp lowering and permissive narrowing on the new ABI | Lossless descriptor-driven mappings and checked conversion | open | T113/T118; legacy migration explicit |

No cleanup task can replace immutable Arrow data with writable shared V8 storage or remove memory/permission checks to achieve a copy count.

## Routine follow-up cleanup obligations — 2026-10-13

| Old path | Replacement | Status | Notes |
| --- | --- | --- | --- |
| Language-word routine dispatch and DDL name/body string splitters | Shared PostgreSQL token/AST path | open | T125/T128; keep only explicitly versioned legacy syntax adapters |
| Lowercased name-only RoutineId keys and dot-split qualified names | Stable identity plus quoted-aware overload lookup | open | T126/T135; durable migration and artifact/grant consumers precede removal |
| Named-input object-key guessing and request/reply JSON envelopes | Explicit binding form, typed native request/reply owners/errors | open | T129/T133/T138; new ABI first, old adapter has one removal milestone |
| Canonical PROCEDURE RETURNS and implicit CALL-return expectations | Functions with SELECT, or procedures with OUT/INOUT | open | T128/T137; inventory and migrate stored and generated callers explicitly |
| Signature metadata duplicated in generated JS and runtime mappings | One revision-pinned call descriptor and generated declarations | open | T134/T135/T137; quoted names and unnamed parameters included |

## CLI development cleanup obligations — 2026-10-13

| Old path | Replacement | Status | Notes |
| --- | --- | --- | --- |
| Independent structural/contract parsing and string rewrites in schema diff | Shared project AST/snapshot and canonical resolver | open | T139; no additional live type registry |
| Concatenated emitters, MODIFY COLUMN, PROCEDURE RETURNS, unsafe type recreation and comment-only unsupported diffs | Typed dependency/risk plan and PostgreSQL emitters | open | T140–T142; preserve explicit legacy migration adapters only |
| Raw-line destructive/section-marker scans | Shared AST/lexer-aware boundaries | open | T142/T144; quoted body text cannot become a destructive operation or marker |
| Current-source baseline copying and filename-only missing-baseline seeding | Target-scoped committed revision receipts | open | T143; retain readers for old baseline metadata through a versioned migration |
| Maximum-mtime watch acknowledgement and generated-output rescans | Bounded input revisions/event tracking and pending-stage retry | open | T146/T153 |
| Repeated generation/artifact/manifest compilation and mixed-revision writes | One staged snapshot with coherent publication/fingerprints | open | T145/T147; never prune user-written handler files |
| Monolithic success labels and migration-only isolated exec | Stage-specific evidence and shared required preparation | open | T149/T150; old agent event compatibility remains explicit |
