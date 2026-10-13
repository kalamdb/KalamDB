# Quickstart: PostgreSQL 18 Compatibility Gates

Commands are the narrow check for a batch. Final stage 7 is the full sweep. Run backend commands from `backend/` unless noted. Use the dev profile. Record the revision and, for performance runs, the runtime in seconds.

## Phase A — baseline

1. Pin the PostgreSQL 18 server version and the client versions in the matrix.
2. Refresh [contracts/cleanup-ledger.md](contracts/cleanup-ledger.md) against the current tree.
3. Run one existing SQL suite and one wire suite and save failures before changing behavior:

```text
cargo nextest run -p kalamdb-dialect
cargo nextest run -p kalamdb-postgres-wire
```

4. Run T096's executable differential/performance harness after it is created; unit tests above are not a performance baseline. Save hardware, resolved Cargo.lock versions, dataset seed, concurrency and raw measurements. Apply [execution-performance.md](contracts/execution-performance.md) targets before behavior changes.

## Phase B — G1 parse once

After the shared statement reaches HTTP SQL and the wire for one query and one parameterized DML:

```text
cargo nextest run -p kalamdb-dialect
cargo nextest run -p kalamdb-core --test prepared_metadata_cache
cargo check -p kalamdb-server
```

Fail the slice if a production path still plans user SQL by parsing text twice, if prepared parameters and results come from two plans, or if production code still parses with a `PostgreSqlDialect` or `GenericDialect` value instead of `KalamDbDialect`. Test-only comparison and rejection parsers are allowed only on the exception list. The dialect tests must show PostgreSQL casts, arrays, dollar strings, nested comments, JSON operators, and quoted identifiers behave the same through `KalamDbDialect` as through the upstream PostgreSQL dialect, except for documented Kalam extensions.

## Phase C — schema batch

One editing batch per invocation, from the repository root for the rewrite script (Cargo commands can run from the workspace root too). T099 identity proof and T031 format decision precede T032. A shared-ID migration may require a dependency-complete integration batch or a temporary adapter; do not claim each edited crate compiles in isolation:

```text
scripts/pg18-rewrite/rewrite.sh --batch backend/crates/<crate> --exceptions specs/037-pg18-compatibility/contracts/schema-migration.md
cargo check -p <crate>
cargo nextest run -p <crate>
```

Search the batch output for `NamespaceId` outside the exception list before starting the next crate. The gate slice is schema create, ordinary table create, lookup, and restart. Do not start that slice until the persisted-format note exists.

## Phase D — type slice

```text
cargo nextest run -p kalamdb-commons
cargo nextest run -p kalamdb-core
cargo nextest run -p kalamdb-serialization
cargo bench --profile dev -p kalamdb-serialization --bench codec
```

The vertical slice is not done until `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array round-trip through storage and text and binary results against PostgreSQL 18. Fail the slice if `ColumnDefinition` still has a second type field that callers treat as authority.

Row storage v2 acceptance requires T097–T102, mixed-format recovery and the quantitative gates in [execution-performance.md](contracts/execution-performance.md). Test historical defaults, failed USING conversion, enum insertion order, array bounds, malformed payloads and selective-buffer retention. Dev profile is explicit because plain `cargo bench` selects an optimized benchmark profile. No runtime speedup is claimed before measurements. Report times in seconds.

## Phase E — execution

Differential fixture for the ordinary application slice: results, types, SQLSTATE, permissions, and commit or rollback. One rollback and one concurrent write are smoke checks only. G4 requires T104/T105 and every required failure/concurrency row in `contracts/execution-performance.md`. Do not skip or weaken an existing assertion to get a pass.

## Phase F — catalogs and clients

```text
cargo nextest run -p kalamdb-views
cargo nextest run -p kalamdb-postgres-wire
```

Compare catalog rows for the fixture schema, table, enum, and routine with PostgreSQL 18 through both SQL entry points. Then run the pinned `psql` and driver checks for connect, query, prepared statement, transaction, cancellation, bulk transfer, and reconnect.

## Phase G — tooling

Run the captured ORM or GUI introspection statements against the shared catalogs. Load the migration fixture and compare typed contents, constraints, indexes, sequence state, schema and row counts with the PostgreSQL 18 source.

## Phase H — release candidate

From the repo root, with a fresh server and a rebuilt CLI:

```text
cargo build -p kalam-cli --bin kalam
```

Then the project sweep in `scripts/test-all.sh`, plus the clean-project journey and an upgrade, backup, restore, restart, and interrupted-operation rehearsal on the actual release artifacts.

CLI smoke, when a server is already up:

```text
cd cli && KALAMDB_SERVER_URL="http://localhost:3000" KALAMDB_ROOT_PASSWORD="mypass" cargo nextest run -p kalam-cli-e2e --test e2e smoke
```

## Rewrite program skeleton

Until `scripts/pg18-rewrite/rewrite.sh` exists, Phase C does not start the mass rename. The first implementation task for that phase is the script, the exception file, and a dry run on `kalamdb-commons` that changes no PostgreSQL catalog identifiers.

## Evidence status

New test targets and the benchmark harness listed in tasks.md are planned until their implementation tasks land. A command is evidence only after execution, with exit status and revision recorded. This review changed Markdown only; it ran document consistency checks, not database compatibility or performance tests. Add measured `#[ntest::timeout(...)]` bounds to new async tests at 1.5 times healthy runtime as required by AGENTS.md.

## Function and V8 gate (G3/G4 extension)

Before behavior changes, T112 records current conversion/copy/CPU/memory baselines using the existing function benchmark target. After the corresponding tasks land, run from the workspace root or backend directory:

```text
cargo nextest run -p kalamdb-functions --test v8_regressions
cargo nextest run -p kalamdb-functions --test runtime_lifecycle
cargo nextest run -p kalamdb-core --lib -E 'test(functions::)'
cargo bench --profile dev -p kalamdb-functions --bench v8_runtime
```

These commands are components of the gate, not proof by themselves. T120 adds the owner-lifetime/copy-count/security regressions; T122 runs query→function→nested-call→return and streamed/concurrent workloads, reporting runtimes in seconds, exact copy boundaries, CPU, allocations, GC and retained native/V8 memory. Compare with baseline under the same ABI semantics; label explicit new read-view/batch APIs separately. Follow [functions-v8.md](contracts/functions-v8.md) and reject a fast path that mutates shared data, loses type precision or exceeds budgets.

T121 also requires generated TypeScript input/output checks and a fresh disposable project exercising SQL/HTTP functions and nested host queries. When backend behavior is implemented, rebuild the CLI and run server-backed smoke/SDK checks before committing under AGENTS.md. No benchmark or runtime checks above were run for this planning-only extension.
