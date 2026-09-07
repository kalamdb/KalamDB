# Stage 8 gate (schema-first functions)

Recorded 2026-09-07 during `/speckit-implement` completion.

## Wasmtime default linkage (SC-018 / SC-019)

`backend/crates/kalamdb-functions/Cargo.toml` keeps `default = ["catalog"]`. `wasm-runtime` is optional and not in default features. Default `kalamdb-server` therefore does not link Wasmtime through the functions crate.

## V8 startup snapshot (T124)

Skipped. Stage 0 did not isolate isolate/init cost from CALL latency, and this machine did not have disk headroom for a release rebuild to measure `CreateParams::snapshot_blob`. No user-revision snapshots were added.

## Benches / RSS / binary size

Stage 0 numbers remain in `specs/034-schema-first-functions/validation/stage0-baseline.md`.

A fresh Stage 8 re-run of comparison benches, idle RSS, and `kalamdb-server` binary size was **not** captured here: local `target/` was already hundreds of gigabytes and the disk was near full. Use the Stage 0 table as the last recorded baseline until a machine with free space re-runs:

```bash
# from specs/034-schema-first-functions/quickstart.md Stage 8
cargo nextest run -p kalamdb-functions
# kalamdb_functions comparison driver + ps RSS + ls -lh target/debug/kalamdb-server
```

## Quickstart Stage 8 commands

Not re-executed in this pass for the same disk constraint. Unit coverage for the remaining product surfaces:

- `kalamdb-functions` lib tests (ABI v2, activation CAS, pool)
- `kalamdb-core` `FunctionRuntimeState` active-run / error ring tests
- `kalamdb-views` `system.active_function_runs` / `system.function_errors` view tests
- CLI `functions` build/bundle tests
- CLI e2e `kobj_functions_active_runs_and_structured_errors` (requires running server)
