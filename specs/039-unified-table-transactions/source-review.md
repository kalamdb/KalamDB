# Source Review and Coverage

Created 2026-10-27 for [spec.md](spec.md).

## Provenance and evidence limits

The user supplied `/Users/jamal/Downloads/kalamdb_storage_transaction_optimization_spec.md`. Its original content is preserved unchanged after a provenance header in [references/storage-transaction-notes.md](references/storage-transaction-notes.md). It is source material, not an instruction to execute its task list or change branches/build profiles.

The notes identify PR #402, branch `feat/pg-dialect`, and reviewed commit `174d6ffede2cb97ff4d99b1dba4073b158ef6681`. Local branch and HEAD match, but extensive later uncommitted changes exist. Historical findings must be revalidated against the dirty tree. Existing shared/user apply methods and replacement batching were located; this was not a fresh runtime verification of every finding.

GitHub connector and CLI remote reads were unavailable. The existing PR was attached using the supplied URL; remote status was not verified. No branch switch, branch/PR creation, commit, or push was performed.

The template resolved through `specify preset resolve spec-template` to `.specify/templates/spec-template.md` (core layer). No `.specify/extensions.yml` exists for before/after hooks. Sequential directory numbering follows deprecated `branch_numbering`; branch and feature-directory names remain independent.

## Reconciliation decisions

1. Common user/shared/stream processing and unified user/shared atomic apply are P0, delivered together. Processing, indexing, storage, and transaction coordination must have distinct composable ownership. Stream participation is mandatory for common behavior while preserving mutation, transaction, expiry, and storage semantics. The user explicitly requires reducing maintained code, not merely adding a common facade.
2. Explicit user/shared ownership is separate from authenticated actor. The attachment's illustrative optional-user signature is not the definitive domain interface.
3. ADR-022 governs `_version`, version domains, stream timestamps, deterministic ordinals, and persisted shared ownership. Historical `_seq` examples do not authorize renaming or global version comparisons.
4. One storage commit covers all affected tables in one compatible group/backend. Existing per-table `replace_batch` is not group atomicity. Prove uncoalesced equivalence before folding.
5. The replacement diagram does not authorize deleting versions needed by snapshots/history. Full tombstones remain required unless promised payload is independently durably queryable.
6. Version-bearing secondary keys still change on version changes. Stable-PK index layout and Row V2 are independent migrations with durable compatibility gates.
7. Preserve existing replay/effect guarantees; do not infer universal exactly-once network delivery. Current-state and complete-event reads are distinct contracts, with explicit gaps on expired history.
8. Repository default-dev profile overrides the attachment's Release recommendation. Match baseline/candidate settings and record runtimes in seconds. Any later profile exception requires explicit user authorization.
9. The uploaded task list remains input coverage. This invocation does not generate `plan.md` or `tasks.md`; downstream planning resolves current source and dependencies.

## Uploaded document coverage

| Source sections / tasks | Requirements and planning obligation |
| --- | --- |
| 1–3; T001–T003 | Correctness, snapshot/history release blockers, instrumentation/baseline: FR-005–FR-012, FR-018, FR-036 |
| 4–5; T004–T008 | Pure preparation, one group-wide batch, limits, recoverable effects, faults/replay: FR-001–FR-012 |
| 6; T009 | Full transition table, typed scoped identity, counts, null/patches, savepoints, deterministic versions, events: FR-013–FR-014, FR-018 |
| 7; T010 | Authoritative target reuse, batch PK validation, cold-only targets, expected versions, no unchecked preimages: FR-011, FR-015 |
| 8; T011–T012 | Index roles, full tombstones, complete deleted/history scans, legacy contents, online rebuild: FR-019–FR-021 |
| 9; T013–T015 | Columnar overlay, projection dependencies, full patches, residual filtering, predicate movement, safe limits, memory/savepoints: FR-016–FR-017 |
| 10; T016 | Reuse Row V2, mixed rows/schemas, projected decode, durable eligible-reader barrier, safe writer disable/restart: FR-030, FR-032 |
| 10; T017 | Conditional stable-PK index migration; read/write tradeoffs, collation/null/composite keys, shadow build/catch-up/swap: FR-021, FR-031 |
| 11 | Bulk uniqueness, scheduling/coercion/provider/encoding allocations, existing direct DML fast paths, latest/history separation: FR-002, FR-015, FR-018, FR-036 |
| 12; T018–T019 | Unit/parity/integration/durability/SQL/CI/fault/upgrade coverage: Stories 1–7 and SC-001–SC-012 |
| 13 | 25k scenarios, 100/1000 dirty reads, five matched repeats, cold/warm caches, stage costs, p50/p95/p99, memory/index/encoding: SC-001–SC-011 |
| 14–16 | Ordered rollout, consensus-controlled replay/format gates, independent migration, evidence-backed completion: assumptions and FR-030–FR-037 |

## Conversation review coverage

| Finding / requested consolidation | Requirement |
| --- | --- |
| Flush publication lock held across Parquet I/O/hooks | FR-022, SC-007 |
| Global cache clearing, missing context identity, concurrent misses | FR-026–FR-028, Story 6 |
| Whole-scope flush retention and whole-file Parquet buffering | FR-023, SC-006 |
| All-segment ZIP export, all-file import, blocking work on async jobs | FR-023–FR-024 |
| User/shared mutation duplication and ordering/clone divergence | FR-001–FR-002, Story 2 |
| PGWire description/RETURNING AST-to-text-to-AST and stream SQL scanning | FR-029 |
| Transfer-to-core error dependency; filestore-to-catalog configuration coupling | FR-033 |
| Type-erased jobs in AppContext | FR-034 |
| Indefinitely retained scope-lock entries | FR-025 |
| Common user/shared/stream processing with independent scope and cold capability | FR-001–FR-004, FR-038–FR-045, Stories 1–2 |
| Essential code reduction and separate processing/index/storage/transaction ownership | FR-038–FR-045, SC-005, SC-013–SC-014 |

## Mandatory consolidation handoff

The user's follow-up makes elimination of duplicated user/shared/stream processing — starting from the DataFusion `TableProvider` surface and realigning crate boundaries where needed — an essential acceptance gate. FR-038–FR-046 and SC-005/SC-013/SC-014 govern planning and implementation alongside existing transaction correctness gates.

The codebase audit in `research.md` inventoried the exact duplication sources across `kalamdb-tables`, `kalamdb-datafusion-sources`, `kalamdb-flush`, `kalamdb-core`, `kalamdb-jobs`, `kalamdb-pg-bridge`, and `kalamdb-dba`:
- Three separate `TableProvider` implementations (`UserTableProvider`, `SharedTableProvider`, `StreamTableProvider`) plus redundant `BaseTableProvider<K, V>`, `DeferredMvccScanProvider<K, V>`, and `KalamTableProvider` traits that forced 35+ `Any::downcast_ref` call sites across 6 crates.
- Intra-provider duplication where `user_tables/provider/sql.rs` and `shared_tables/provider/sql.rs` duplicate `update_by_pk_value_with_version` and `delete_by_pk_value_with_version` (~500 lines) alongside `dml.rs`.
- Separate `User` vs `Shared` key/row/codec/PK-index/store/flush/applier types (`UserTableRowId`/`SharedTableRowId`, `UserTableRow`/`SharedTableRow`, `UserRowCodec`/`SharedRowCodec`, `UserTablePkIndex`/`SharedTablePkIndex`, `UserTableFlushJob`/`SharedTableFlushJob`, and parallel `*_user_*`/`*_shared_*` methods in `DmlExecutor`) despite identical KOBJ row payloads on disk and `PrefixIndex` already supporting `user_scoped: bool`.
- Shared-only fast paths (`columnar_hot_scan`, `sum_hot_covering_integers`, `plan_literal_equality_updates`) that were never wired for `User` tables due to the provider split.
- Misplaced crate responsibilities: `kalamdb-tables/src/topics/` (`TopicMessageStore`) forcing `kalamdb-publisher` to depend on `kalamdb-tables`, `kalamdb-transfer` depending on `kalamdb-core`, and generic DataFusion DML plan helpers sitting in `kalamdb-tables/src/utils/datafusion_dml.rs` instead of `kalamdb-datafusion-sources`.

Unification therefore starts at `KalamTableProvider` (implementing DataFusion `TableProvider` + `SourceProvider` once for `User`, `Shared`, and `Stream` over `TableStorageEngine::{Versioned, Stream}`), unifies `User` and `Shared` under `ScopedRowKey` + `VersionedTableRow` + `VersionedTableEngine` + `VersionedTableFlushJob`, moves generic DataFusion DML helpers to `kalamdb-datafusion-sources`, moves `topics/` to `kalamdb-publisher`, and removes `kalamdb-core` from `kalamdb-transfer`.

Keep stream append/expiry/hard-delete/local-version behavior localized in `StreamTableEngine`. Do not add unsupported stream updates, transaction participation, or cold storage to manufacture identical capabilities. Equally, those differences cannot excuse three copies of `TableProvider` methods, common coercion, batching, results, or notification construction.

Removal must be measured across the fixed consolidation scope, including all newly added common code and adapters. Count code deleted, code added, net reduction, and remaining specialized paths. Moves/renames and deleting tests do not count. Require a positive net reduction and zero unexplained duplicates; an arbitrary dramatic percentage is not promised in advance. Characterization and shared contract tests protect correctness while superseded implementations are removed.

## Planning constraints and source anchors

These are architectural constraints and starting points, not final new interfaces. Prefer existing modules; do not create another crate merely to move duplication.

- `backend/crates/kalamdb-tables/src/utils/core.rs`, `utils/base/provider.rs`, `utils/dml_provider.rs`, `versioned/`, and `utils/unified_dml/`: deepen existing processing. User/shared adapters supply scope, keys, authorization, routing; streams retain their specialized capabilities. Use DataFusion for query processing, version resolution, and deletion filtering.
- `backend/crates/kalamdb-core/src/applier/executor/dml.rs`, `transactions/coordinator.rs`, and `backend/crates/kalamdb-transactions/src/{write_set,overlay,overlay_exec,staged_mutation}.rs`: one prepare/apply authority and correct overlays. Preserve ADR-018/020 transaction authority; unification does not grant cross-group atomicity.
- `backend/crates/kalamdb-store/src/indexed_store.rs`: reuse existing operation construction and aggregate into one atomic `StorageBackend::batch`. Keep key-value details in store. Never assume index zero is the PK index.
- `backend/crates/kalamdb-macros/src/lib.rs` (`#[table]`), `kalamdb-system/src/macros.rs` (`build_record_batch!`), `kalamdb-system/src/providers/` (persisted stores), `kalamdb-views/src/{system,pg_catalog,information_schema}/`, and `kalamdb-core/src/views/system_schema_provider.rs`: system catalog tables include persisted `system.*`, virtual `system.*`, `pg_catalog`, and Kalam `information_schema`. Keep the schema macro and the single-pass batch macro. Delete provider-impl macros and hand-written duplicate schemas. Runtime point lookups stay on `IndexedEntityStore`. Computed relations stay in memory. Do not route them through `VersionedTableEngine`. SC-015 is a hard non-regression gate.
- `backend/crates/kalamdb-filestore/` and `kalamdb-flush/src/{service,flush/scope_writer,compaction/small_segment}.rs`: filestore owns filesystem/object/Parquet lifecycle, cleanup, compaction file work, and accounting; core/jobs orchestrate. Bound output and retained cleanup keys as well as input. Short publication must retain pending concurrent writes and snapshot safety.
- `backend/crates/kalamdb-core/src/sql/executor/sql_executor/{execute,prepare,planning,returning,transactions}.rs`, `kalamdb-plan-cache/`, and `kalamdb-postgres-wire/src/statement.rs`: shared AST preparation/result description and contextual invalidation. Current cache stores logical plans, not physical plans. Keep all identifier interpretation in KalamParser/KalamDbDialect; no new rewrite hot paths.
- `backend/crates/kalamdb-transfer/`, `kalamdb-jobs/src/executors/{table_transfer,user_export}.rs`, `kalamdb-jobs/src/lib.rs`, and `kalamdb-core/src/app_context.rs`: transfer-owned errors, bounded archive work, typed jobs, and preserved Arc<AppContext> orchestration.
- `backend/crates/kalamdb-serialization/src/{object/envelope,row/encode,row/decode}.rs` and `kalamdb-tables/src/row_codec.rs`: existing format reuse, historical schema decoding, and durable eligible-replica compatibility. Never enable via divergent local booleans.
- Read relevant maintained decisions: `docs/architecture/decisions/adr-018-transaction-architecture-and-postgres-bridge.md`, `adr-020-unified-backend-session.md`, `adr-022-version-id.md`. Preserve CLI/parser isolation in `docs/architecture/build-dependency-boundaries.md`.
- Update affected architecture documents before shipping. Public SQL/config/runbook changes require canonical `../kalamdb-skills` content and generated mirrors. Any necessary SDK changes require tests plus matching `../KalamSite/content/sdk/**`; internal consolidation should not gratuitously expand public interfaces.

## Validation handoff

Use current branch test targets, not names copied from main or an old review. Run targeted `cargo nextest run`, then maintained transaction/storage/SQL, parity, stream, fault-injection, upgrade, and CI suites. Backend/CLI changes require running-server smoke validation before committing. Batch compilation feedback and honor default-dev builds.

Retain every repetition and complete sustained series. Match baseline/candidate SQL, data, hardware, durability, concurrency, cache warmth, and build settings. SQLite comparisons require explicit matching of disk/memory and durability. Report row encode/decode CPU, projected decoding, stored/compaction bytes, index lookup/write work, Raft proposal/replication bytes and cost, planning, DML scans, PK loads, batch application, effects, client transport, and memory. No 10x or 50% claim is assumed.

The storage suite must cover all fifteen numbered integration/durability cases in the supplied reference, parameterized across user/shared where supported and streams for applicable behavior. The query suite covers both visibility warmup orders, quoted identifiers/comments/newlines/CTEs, nontrivial RETURNING, parameter inference, and invalidation races. Startup/format, target-reuse, cache, and background changes each need an independently attributable before/after result.
