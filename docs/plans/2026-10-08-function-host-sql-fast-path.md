# Function host SQL result path

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Hand `ctx.db.query` / `ctx.db.execute` results to JavaScript without rebuilding an Arrow batch, and leave transactions, permissions, Raft, and SQL behavior unchanged.

**Architecture:** Nested host SQL already calls `SqlExecutor` in-process. The plan cache and the primary-key point-get path already run. The waste is on the way back: a point get returns `ExecutionResult::ScalarRows`, and `execution_result_to_rows` / `execution_result_to_routine` call `into_arrow_rows()`, which turns those scalars into JSON, then into a `RecordBatch`, then back into scalars. Convert `ScalarRows` directly into the same `RoutineValue` shape V8 already receives. Queries that are not point gets stay on the existing Arrow path.

**Tech stack:** Rust, `kalamdb-core` function host, existing `SqlExecutor`, `RoutineValue` / V8 struct conversion.

---

## What this change is

On the 2026-10-08 bake-off, a point read is 200 µs through `POST /v1/api/sql` and 1.21 ms through `POST /v1/functions/...`. An insert is 357 µs versus 1.70 ms. The nested statement does not make a second HTTP call. Most of that gap is the procedure shell: catalog checks, the request transaction, the hop onto the V8 worker, and the spawn back onto the server runtime.

The Arrow rebuild is still the wrong direction. `ExecutionResult` documents it as a measured regression on the HTTP path (about 16.9 s versus 15.3 s for 1 million reads). Functions take that path on every `ctx.db.query`. This change removes it.

Expect a modest drop on the functions read column, not SQL-level latency. Re-run the comparison after the tests pass. If the functions read wall time barely moves, stop. Do not add a second shortcut in the same change.

## What this change must not do

These stay as they are. Each one is a behavior change, not a cleanup.

- The procedure still begins and commits its request transaction, including a read-only body. Nested statements still share that transaction.
- JavaScript still runs. Do not detect a "trivial" procedure body and skip V8.
- Inserts, updates, and deletes still go through `SqlExecutor` and Raft. Do not add a direct store write from the host.
- Host SQL still uses `Handle::spawn` onto the server runtime. `block_on` from the V8 worker can deadlock when DML fires a table trigger or the statement is `CALL`, because that work needs the worker that is blocked.
- HTTP `/v1/api/sql`, pgwire, live queries, and cluster forwarding stay on their current result paths. `into_arrow_rows` remains for those callers.
- `EXECUTE AS` parsing stays. A normal statement already returns at the prefix check.

## Result shape to preserve

`ctx.db.query` uses `execution_result_to_rows`. One row or many, the value is a `ScalarValue::List` of one-row structs, in schema field order. Zero rows is an empty list. A missing projected key is null. Aliases are already the `Row` keys; do not iterate the `BTreeMap` (that sorts by name and breaks `SELECT name AS title`).

`ctx.db.execute` uses `execution_result_to_routine`. DML stays `Int64` rows-affected. A one-row, one-column read stays a bare scalar. A one-row, multi-column read stays one struct. Many rows stay a list. `Inserted` / `Updated` / `Deleted` already pass through `into_arrow_rows` unchanged, so insert latency will not move from this conversion.

The lock for "same JavaScript value" is a test that converts one `ScalarRows` value both ways and compares the `RoutineValue`s: today's `into_arrow_rows` path versus the direct path. Column order, nulls, aliases, `Int64`, and `Utf8` must match. Direct conversion then replaces the Arrow call. The Arrow path remains in the test as the oracle until that test is updated to pin the struct shape itself.

Row and byte limits stay: more than 10,000 rows, or more than 8 MiB of result memory, is still `procedure query result limit exceeded`. Apply that check to `ScalarRows` before building structs. For the byte check, sum `ScalarValue::size` across columns. Do not build the Arrow batch just to measure it.

## Files

- Modify: `backend/crates/kalamdb-core/src/functions/convert.rs`
- Test: `backend/crates/kalamdb-core/src/functions/convert.rs` (`mod tests`)
- Do not modify: `backend/crates/kalamdb-core/src/functions/host.rs` (`run_sql` already calls the two converters), `backend/crates/kalamdb-core/src/sql/context/execution_result.rs`, the SQL HTTP serializer, or the comparison drivers.

## Task 1: Lock the current point-get shape

**Step 1.** In `convert.rs` tests, build a `Schema` with fields `id` (`Int64`) and `data` (`Utf8`), in that order. Build `ExecutionResult::ScalarRows` with one `Row` whose map is inserted as `data` then `id`, so map order and schema order differ. Include a second case with zero rows, and a third with two rows.

**Step 2.** Convert each with the current `execution_result_to_rows` (this still goes through Arrow). Assert:

- one row: a list of one struct whose fields are `id`, then `data`
- zero rows: an empty list
- two rows: a list of two structs, schema order

**Step 3.** Run:

```bash
cargo nextest run -p kalamdb-core --lib functions::convert
```

Expected: the new assertions pass against today's converter. If they fail, the assumed shape is wrong; fix the assertions to match the current converter before changing production code.

## Task 2: Convert ScalarRows without Arrow

**Step 1.** Add `scalar_rows_to_routine_list(rows, schema) -> Result<RoutineValue, KalamDbError>` in `convert.rs`. For each row, walk `schema.fields()` and take `row.values` by field name, using `ScalarValue::Null` when the key is absent. Build the same one-row `ScalarValue::Struct` that `scalar_struct_from_row` builds. Wrap the structs in `ScalarValue::List` with the struct's data type, matching the empty-list and non-empty-list behavior of `execution_result_to_rows`.

**Step 2.** In `execution_result_to_rows`, match `ExecutionResult::ScalarRows` before `into_arrow_rows`. Enforce the 10,000-row and 8 MiB limits, then call `scalar_rows_to_routine_list`. Leave `ExecutionResult::Rows` on `scalar_struct_from_row`.

**Step 3.** In `execution_result_to_routine`, match `ScalarRows` the same way and then apply the existing one-cell / one-row / many-row rules from `rows_to_routine` to that list. DML variants stay on their current match. Do not call `into_arrow_rows` for `ScalarRows`.

**Step 4.** Extend the Task 1 tests:

- `execution_result_to_rows` on `ScalarRows` equals the oracle recorded in Task 1
- a schema alias field (`title`) reads that key, and a missing key becomes null
- 10,001 rows returns the existing limit error and does not allocate the struct list
- `execution_result_to_routine` on a one-column `ScalarRows` point get returns the bare scalar, and on an `Inserted { rows_affected: 1 }` returns `Int64(1)`

**Step 5.** Run the same nextest command. Expected: pass.

## Task 3: Confirm the neighbors did not move

Run:

```bash
cargo nextest run -p kalamdb-core --lib functions::
cargo check -p kalamdb-core -p kalamdb-functions -p kalamdb-api
```

Expected: function host, conversion, and executor tests pass. No change to HTTP SQL golden tests is required, because those callers never use `execution_result_to_rows`.

## Task 4: Measure, then stop

Re-run only the two KalamDB tracks, against the binaries already pinned for 2026-10-08:

```bash
cd benchv2/comparison
KALAMDB_PORT=2920 ./scripts/run-kalamdb.sh
KALAMDB_PORT=2920 ./scripts/run-kalamdb-functions.sh
```

Use port 2920 when a dev server is already on 2900. Record both result files. Compare functions read wall time and read p50 with `results/2026-10-08-apple-m5-pro.md` (14.01 s / 200 µs SQL, 80.61 s / 1.21 ms functions).

Write the numbers into a short note under `benchv2/comparison/results/`. Do not retune TrailBase, PocketBase, or SurrealDB for this measurement.

If functions reads are only a few percent faster, that is the expected result. Ship the conversion anyway: it deletes a known-bad rebuild and keeps the result shape tested. Do not start a V8 bypass, a transaction bypass, or a direct storage API in this change.

## Later, only if a follow-up is explicitly requested

The remaining gap is the procedure shell. A follow-up has to treat these as separate designs, each with its own tests:

- Caching `get_routine` / `list_parameters` for a hot revision, with catalog invalidation, so a call does not reread the catalog.
- Keeping the V8 worker and still running host SQL on the server runtime. Any design that blocks the worker while SQL runs has to prove triggers and `CALL` cannot need that same worker.

Neither belongs in the implementation of this plan.
