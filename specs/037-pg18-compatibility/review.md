# Second review — PostgreSQL 18 compatibility

**Latest follow-ups**: 2026-10-13. See the scoped CLI/routine reviews below, [cli-development.md](contracts/cli-development.md) and [routines-postgresql.md](contracts/routines-postgresql.md). Earlier observations retain their original inspection date; do not treat them as current implementation status.

**Date**: 2026-10-11 · **Inspected code**: `be5efdd2` plus the current untracked feature documents · **Scope**: planning review and corrections only.

The direction is sound: one parsed representation, one logical type authority, shared catalogs and deletion of superseded paths. The original plan was not ready for implementation unchanged. The most serious issues were schema-evolution correctness, identity migration and unbounded ownership hidden behind the zero-copy goal. Corrections are now integrated into spec, plan, data model, contracts, tasks and quickstart. This review changed planning documents only, ran no benchmark, and makes no runtime compatibility/performance claim.

## CLI development follow-up — 2026-10-13

Inspected current CLI/dialect sources based on `9cf03307` and the 035 lifecycle contract. Coverage exists, but T023/T038 and the broad T121/T137 journey do not establish a complete PostgreSQL schema-diff/development lifecycle. This review changes only planning documents; the following source mechanisms require implementation/regression work, not claims that every runtime path was reproduced.

| Severity | Inspected mechanism / gap | Added obligation |
| --- | --- | --- |
| P1 | `kalam-schema-diff/src/diff.rs` emits structural tables first, then separately compiled type/procedure DDL. `contracts/diff.rs` drops types before dropping routines. | One dependency-ordered operation graph, including reverse drops and transaction boundaries. T139/T142. |
| P1 | `contracts/compile.rs::apply_statement` recognizes procedures but falls through successfully for unhandled SQL; the structural parser ignores other AST variants. | CREATE FUNCTION/DOMAIN and other required forms cannot silently disappear. T139–T141. |
| P1 | `emitter/modify_column.rs` emits MODIFY COLUMN; routine emission still uses PROCEDURE RETURNS; constraint changes become comments. | PostgreSQL emitters and typed blocked/manual outcomes; no baseline advancement for comment-only differences. T140/T141. |
| P1 | Diff compilation defaults to public, codegen uses project.default_env, apply resolves the selected environment; `normalize_object_key` splits dots and lowercases quoted names. | One resolved target/search path and quoted-aware source model. T139/T148. |
| P1 | `dev/watch.rs` seeds a missing baseline from current schema.sql when filenames are applied, copies the latest file after apply, and can update baseline in generation-only mode. | Exact committed source/receipt, scoped by target/database identity, with drift and fresh-clone reconstruction. T143. |
| P1 | Contract type/routine removals do not receive structural diff's allow-destructive flag; `workflow/agent.rs` only scans lines for selected DROP forms. | Typed safety classification across all objects and explicit migrations; force cannot bypass stale-target checks. T142. |
| P1 | `migration/apply/mod.rs` saves applying/applied records around `execute_sql_batch`; `workflow/sql.rs` sends individual statements unless a file supplied an explicit transaction. | Prove transaction/receipt and multi-phase recovery, concurrent-client coordination and unknown commit handling. T144. |
| P1 | `migration/markers.rs` recognizes marker-looking lines without SQL lexical state. | Preserve `-- DOWN`/`-- UP` inside quoted routine bodies. T144. |
| P1 | `dev/orchestrator.rs` can build/activate after a Paused schema result; builds reread the schema independently. | Preflight and publish one compatible revision, explicit partial-state recovery and activation checks. T145/T147. |
| P2 | Function watch uses the maximum mtime, scans generated/build files, and records the latest stamp even after failure. Schema watching/baseline copying assumes a file while codegen supports directories. | Bounded revision-aware directory watch, no feedback loops/lost edits, transient retry and matching source support. T139/T146. |
| P2 | Function generation, artifact and manifest compile the project separately; missing tsc returns success. | Shared snapshot/fingerprint, required-tool diagnostics, staged output publication and no-op caching. T147. |
| P2 | `dev/exec.rs` applies migrations/seed without generation/build/activation; readiness output has no active-function revision and app health is separate. | Full isolated JS preparation, target isolation and honest stage reporting while preserving 035 ownership behavior. T148–T150. |
| P2 | Current CLI tests include useful fake-server migration tracking but that cannot validate PG DDL dependencies or real V8 calls. | Disposable real-server journey, failure/concurrency matrix and CLI CPU/memory/edit-loop evidence. T151–T153. |

T139–T153 are appended without changing completed tasks. The new contract names supported/manual/gap outcomes, bounded watch work and migration/application failure states. Implementations and runtime measurements remain outstanding. Existing back-end/CLI work in the checkout was preserved.

## Routine follow-up — 2026-10-13

Inspected the working tree based on `9cf03307`, including active uncommitted implementation changes. The type/routine surface is **not yet PostgreSQL-aligned end to end**. Existing G1/schema task completion does not close the following scoped gaps. No implementation files were changed by this review.

| Severity | Current evidence / design flaw | Correction and new task |
| --- | --- | --- |
| P1 | `ddl/create_procedure.rs` lowercases quoted names, accepts RETURNS and parses only untagged dollar bodies; `classifier/engine/core.rs::is_kalam_procedure` selects a path by language words. | Shared AST for PostgreSQL routine grammar irrespective of JS language; preserve names/body spans and true function/procedure distinction. T125/T128. |
| P1 | `models/ids/routine_id.rs::from_parts` lowercases a schema-qualified string without input signature; catalog routine identity is the primary key. | Stable RoutineId plus canonical overload key; function/procedure kind cannot bypass signature conflicts. T126/T135. |
| P1 | `ddl/create_type.rs::take_ident` stops at the first double quote; enum validation forbids empty/colon labels and measures characters; empty definitions are rejected. | PG lexer/resolver, exact label/byte rules, composite/table-row-type semantics. T125/T127 supplement T124. |
| P1 | `DropProcedureStatement::parse` ignores text after the name, while `CALL` strips GUI LIMIT/OFFSET clauses; signature and trailing-token semantics diverge. | Signature-aware lifecycle and canonical CALL grammar, with explicit legacy migration where needed. T125/T128/T129. |
| P1 | `core/functions/executor.rs::pack_named_call_input` guesses named bindings by object keys, packs JSON and substitutes conversion failure with NULL. | Explicit binding forms and typed request/reply messages, including one composite argument and exact errors. T129/T133. |
| P1 | The plan specifies broad routine attributes/results but lacks a concrete SQL expression/set/table bridge into async V8 and procedure output mapping. | DataFusion integration proof, bounded dispatch, output cardinality and context semantics. T130/T131/T134. |
| P2 | Pending T043 said to keep function typmods, contradicting FR-073 and the PG reference. | Corrected that pending description to discard signature typmods, preserving applicable field/domain constraints. T127/T128 provide context coverage. |
| P2 | Language parsing, JS host APIs and SQL-visible behavior could be conflated into a compatibility claim. | JS/TS are declared Kalam languages with normal SQL call shapes; SQL/PLpgSQL retain their own execution tests. T132/T136. |
| P2 | The existing V8 plan covers buffer ownership but not explicit no-envelope counters for both requests and replies, or overload-aware generated artifact bindings. | Per-boundary evidence and migration of CLI/SDK/stored consumers. T133/T135/T137/T138. |

Completed task descriptions/statuses were preserved. Pending T043/T050 were corrected; T125–T138 were appended. These are design resolutions and implementation obligations, not measured fixes. The review checked current source and the linked PG18 documentation; it did not run the PG oracle, backend tests or performance benchmarks.

The developer experience target is ordinary PostgreSQL type/routine declarations and calls, with JS/TS selected by LANGUAGE. Internally, typed owners remove avoidable invocation serialization. Arbitrary mutable JS objects still require conversion; network/durable encodings and legitimate JSON values still have codec costs. Sources and acceptance examples are in [the routine contract](contracts/routines-postgresql.md).

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
