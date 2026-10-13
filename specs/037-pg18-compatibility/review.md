# Second review — PostgreSQL 18 compatibility

**Date**: 2026-10-11 · **Inspected code**: `be5efdd2` plus the current untracked feature documents · **Scope**: planning review and corrections only.

The direction is sound: one parsed representation, one logical type authority, shared catalogs and deletion of superseded paths. The original plan was not ready for implementation unchanged. The most serious issues were schema-evolution correctness, identity migration and unbounded ownership hidden behind the zero-copy goal. Corrections are now integrated into spec, plan, data model, contracts, tasks and quickstart. No server code was changed, no benchmark was run, and no runtime compatibility/performance claim is made.

## Findings and resolutions

| Severity | Finding in the original design | Resolution / implementation gate |
| --- | --- | --- |
| P1 | Reading absent old columns using the current default changes historical data after SET DEFAULT. Blanket cast-on-read postpones ALTER TYPE errors and can repeat conversion effects. | Historical missing values; transactional DDL validation/backfill; T101 and revised row-storage contract. |
| P1 | NamespaceId is a lowercased name and TableId embeds it. Renaming the wrapper cannot provide stable identity or quoted-name compatibility. The spec also rejected all case-different column names. | Separate immutable IDs from resolved names; preserve quoted distinctions and restart/move behavior before mass renaming; T099. |
| P1 | Enum ordinals change when labels are inserted; packed composite arrays omit dimensions, lower bounds and variable-element layout. | Stable member IDs and explicit array/nested revision metadata; T100. |
| P1 | A new KOBJ flag was treated as sufficient compatibility protection although current readers do not reject unknown flag masks. | Reader-first deployment, durable activation barrier, unsupported-format refusal and downgrade/backup policy; T097. |
| P1 | The proposed decoder trusted persisted UTF-8 and envelope length; recursion, offsets, expansion and integer bounds were underspecified. | Checked boundary validation and allocation limits before exposing trusted views; malformed-data tests in T102. |
| P1 | Keeping all schema versions without a RAM/recovery policy risks unbounded memory or deleting a layout still referenced by data. u16 revisions can exhaust. | Durable history with reachability rules, bounded Arc caches, nested dependencies and fail-before-wrap behavior; T101. |
| P1 | “One plan” did not define dependency invalidation, parameter OIDs, principal binding or session state. Existing cache capacity is entry-based. | One syntactic parse, revision-aware analysis, safe cache keys and byte admission; T103/T107. |
| P1 | G4 task closure required only an ordinary fixture, rollback and one concurrent writer, far less than the spec's transaction promise. | Explicit implementation/test matrix for isolation, savepoints, constraints, ambiguous commit, DDL and live events; T104/T105. |
| P1 | Shared SELECT/flush conversion and direct Arrow writes could bypass DML checks or discard versions/tombstones if callers share SELECT visibility. | Shared codec primitives with distinct caller semantics; atomic validation, ownership and retention tests; T102. |
| P2 | “Only one copy” ignores primitive materialization, short inline values and conversion. A tiny result can retain a large view buffer. | Compare view and compact output, bound owner bytes and active work; T098/T108. |
| P2 | No credible PG performance experiment: unit-test runs were called baselines, every codec case was required to improve, and plain cargo bench violated the dev-profile requirement. | Preregistered workload targets, dev-profile benchmarks, CPU/allocation/tail/memory measurements and prior-KalamDB/PG ratios; T096/T108. |
| P2 | Out-of-row pointers were proposed without atomicity, replication, snapshot GC or recovery design. | Remove them from initial v2 scope; inline limits are explicit and a future large-value path needs its own proof. |
| P2 | Wire coverage omitted protocol error recovery and portal state; migration compared only counts. | State-machine coverage and typed-content/sequence/index comparisons; T106/T109. |
| P2 | Routine grammar tasks could be mistaken for language runtime delivery; crate-by-crate ID deletion could leave dependent crates uncompilable. | Runtime acceptance in T110; dependency-complete integration batches or tracked temporary adapters. |

P1 fixes are implementation prerequisites, not observed production incidents. All listed fixes are **resolved in design; implementation pending**.

## Current-code evidence

- `backend/crates/kalamdb-commons/src/models/ids/namespace_id.rs`: constructors lowercase names; `table_id.rs`: namespace and table name compose the key.
- `backend/crates/kalamdb-serialization/src/row/schema.rs`: u16 schema revision, fields, one-dimensional List; `row/batch.rs`: intermediate ScalarValue vectors and clones; existing input/nesting limits.
- `backend/crates/kalamdb-serialization/src/object/envelope.rs`: envelope version/length checks and flag extraction, without a supported-bit-mask check.
- `backend/crates/kalamdb-plan-cache/src/lib.rs`: PlanCacheKey contains namespace, role and SQL; capacity uses entry counts.
- `backend/crates/kalamdb-postgres-wire/src/statement.rs`: separate text planning for parameter and result inference.
- `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs`: text planning and retry paths still exist; point-get optimization already exists and must be preserved.
- `backend/crates/kalamdb-dialect/src/ddl/create_table/parser.rs`: ordinary table creation currently defaults to Shared.

These observations justify targeted tasks; they do not prove every existing execution path has a bug.

## Reference checks

PostgreSQL's identifier distinctions establish why blanket lowercasing must be removed. [Lexical structure](https://www.postgresql.org/docs/18/sql-syntax-lexical.html).

DDL behavior and enum identity confirm that schema-history decoding cannot be based on mutable defaults or sort positions. [ALTER TABLE](https://www.postgresql.org/docs/18/sql-altertable.html), [pg_enum](https://www.postgresql.org/docs/18/catalog-pg-enum.html).

Arrays, prepared invalidation and protocol recovery require value/session state beyond syntax recognition. [Arrays](https://www.postgresql.org/docs/18/arrays.html), [PREPARE](https://www.postgresql.org/docs/18/sql-prepare.html), [message flow](https://www.postgresql.org/docs/18/protocol-flow.html).

Arrow views can retain unneeded backing storage; measure and reclaim it at suitable boundaries. The online API is explanatory only; T002 checks the exact resolved workspace version before selecting APIs. [Apache Arrow GenericByteViewArray](https://docs.rs/arrow/latest/arrow/array/struct.GenericByteViewArray.html).

## Remaining decisions and proof

T096 fixes concrete workload sizes and memory budgets before implementation. T097 fixes deployment topology and activation mechanics; T099 fixes the persisted-ID transition; T100/T110 record any unsupported types or languages without marking them verified. These are bounded engineering decisions with tests, not implicit product exclusions. No format switch passes until its measurements and recovery evidence exist.

The plan now includes measurable improvement targets, not a promise to beat PostgreSQL. No finite review can cover every SQL program; the differential harness, malformed-input tests, explicit matrix gaps and concurrency schedules make omissions visible and reproducible.

## Follow-up: function calls, query objects and V8

The user requested this integration after the initial 14-finding review. [functions-v8.md](contracts/functions-v8.md) now defines one descriptor/owner path from Arrow query results through native calls, V8 read views, child routines and returned data. Added FR-160–FR-167, SC-012/SC-013 and T112–T122; all implementation remains pending.

Additional inspected concerns: per-cell Arrow/scalar reconstruction, named-input JSON packing, nested parameter clones, binary copies, microsecond timestamp truncation at V8 conversion, and accounting for imported/pinned buffers. The existing converter already has a direct typed path, and encoded transfer caches are not used by V8; the plan preserves that useful foundation.

The selected design distinguishes readonly native owner forwarding, explicit mutable materialization, exclusive byte-buffer transfer, and unavoidable string/object/output conversions. Child realms and permissions remain unchanged. New view/transfer/batch behavior is versioned and generated; existing ordinary JS array/object mutation semantics are preserved. Buffer aliasing, slice disclosure, allocator/sandbox restrictions, stale handles, async lifetime and delayed GC have explicit tests and memory gates.

No claim that all JS objects can alias Arrow or that all boundaries become zero-copy. Eligible forwarding must prove zero payload copies; whole-path performance must separately meet allocation/CPU/latency/memory targets. Exact production build flags and runtime evidence are still required.
