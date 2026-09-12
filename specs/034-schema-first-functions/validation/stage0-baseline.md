# Stage 0 baseline (before V1.1 behavior refactors)

Captured 2026-09-07. Local check/test used the default (non-release) profile unless noted.

## Unit tests

```text
cargo nextest run -p kalamdb-functions
13 passed, 0 failed, 0 skipped
runtime ≈ 0.236s after compile
```

`kalamdb-core -- functions` was not isolated in this capture; `kalamdb-functions` is the runtime crate gate for Stage 0.

Compile of `kalamdb-functions` with **default features still pulled Wasmtime** (`default = ["catalog", "wasm-runtime"]`). T003 removes that from the default feature set.

## `kalamdb-server` binary size

Existing artifacts on this machine (not rebuilt for this note):

| Profile | Path | Size |
|---------|------|------|
| debug | `target/debug/kalamdb-server` | 687M (mtime 2026-09-07 00:25) |
| release | `target/release/kalamdb-server` | 156M (mtime 2026-09-07 10:23) |

Release size is recorded for SC-020 comparison only. Implementation work stays on the default profile.

## Idle RSS

No idle `kalamdb-server` process was running during capture. Record on a fresh start with no CALL:

```bash
ps -o rss=,command= -p "$(pgrep -n kalamdb-server)"
```

RSS is in KiB (`ps -o rss=`).

## Functions comparison bench (existing)

Source: `benchv2/comparison/results/kalamdb-functions-20260907-102418.txt`  
Server: `target/release/kalamdb-server`  
Timed path: `POST /v1/functions/bench/{insert_message,get_message}`  
Nested SQL: ABI v1 `ctx.db.sql` interpolated literals.

| Metric | Runtime |
|--------|---------|
| Insert 100000 rows | 12.058447291s |
| Insert 10000 rows | 1.304092167s |
| Write p50 / p95 | 1.902292ms / 2.842417ms |
| Read 1000000 rows | 105.693183792s |
| Read p50 / p95 | 1.6825ms / 2.230042ms |

Do not treat these as a V1.1 success gate until Stage 7 re-runs the same driver after the pool/ABI work.
