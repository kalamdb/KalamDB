# T002 Baseline Measurements & Harness Record

**Recorded:** 2026-10-23  
**Baseline commit:** `e3d6f419268b273f81ba75ca0a4ab3b33880e1f4`  
**Toolchain:** `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `aarch64-apple-darwin`  
**Profile:** workspace `dev` / `test` (`opt-level = 0`, `debug = "line-tables-only"`)

## 1. Pre-Change Suite Runtimes

| Suite | Command | Tests Run | Passed | Skipped | Wall Time (s) |
| --- | --- | --- | --- | --- | --- |
| `kalamdb-serialization` + `kalamdb-dialect` + `kalamdb-functions` | `cargo nextest run -p kalamdb-serialization -p kalamdb-dialect -p kalamdb-functions` | 845 | 845 | 2 | 2.086s |
| `link-common` (`client-core`) | `cargo nextest run -p link-common --features client-core` | 124 | 124 | 0 | 0.080s |

## 2. Shared Fixtures & Harness Entry Points (`link/tests/sdk-contracts`)

- `link/tests/sdk-contracts/check_generated.py` — Disposable project harness compiling generated TypeScript (`tsc --strict --noEmit`) and Dart (`flutter test` + negative `dart analyze`) against local SDK packages.
- `link/tests/sdk-contracts/fixtures/schema.sql` — Base table/enum/composite/array/FILE contract fixture.
- `link/tests/sdk-contracts/fixtures/canonical_contract.sql` — Extended Gate A contract fixture covering ordered composite primary keys, identity/generated/default columns, domains, recursive composite/array types, routine parameter modes (`IN`/`OUT`/`INOUT`/`VARIADIC`), overloads, and `VOID`/`SCALAR`/`SETOF`/`RETURNS TABLE` cardinalities.
- `link/tests/sdk-contracts/fixtures/adversarial_vectors.json` — Shared positive, negative, and hostile-frame vectors for scalar bounds, nullability states (`omitted`, `sql_null`, `json_null`), preallocation attacks, recursion depth, null floods, and decompression limits.
- `link/tests/sdk-contracts/workload-manifest.json` — Pre-registered workload weights, latency SLOs, candidate list (`kalam-packed`, `postcard`, `prost`), and promotion thresholds for Gate D and Gate R.
- `link/tests/sdk-contracts/t051-inventory.md` — Static code inventory vs. runtime evidence across execution, binder, serialization, Link/WASM/FRB, and external format boundaries.
