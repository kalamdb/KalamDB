# Build dependency boundaries

The CLI and SQL dialect must not depend on the system-table providers, RocksDB
store, or DataFusion execution engine. They share `KalamParser` and its AST with
the server. Arrow and `datafusion-common` remain available for shared schema and
scalar representations.

`kalamdb-commons::models` owns `StorageMode`, `StorageType`, `VectorMetric`, and
the shared identifier types. `SystemTable` is also exported by commons. System
providers retain their existing public re-exports so callers use the same types.

Schedule validation and next-run calculations live in
`kalamdb-commons::helpers::schedule_timing`, behind the `scheduling` feature.
Both the dialect and system catalog enable that feature. The system catalog
retains its prior helper path as a re-export. Parsing therefore validates the
same schedule rules without compiling catalog providers or the storage engine.

`scripts/check-cli-dependency-boundary.py` checks the CLI's resolved normal and
build dependencies. Main and cluster CI run this check before building.

CI builds normal executables and tests together under the test profile with
matching package selections, preventing an earlier CLI-only build from compiling
shared dependencies with a different feature set. Main CI retains dependency
caches after test failures and versions the native cache by the RocksDB compiler
wrapper instead of deleting RocksDB artifacts on every run. Both workflows
upload Cargo compilation timings for before/after comparisons.

These boundaries reduce compilation scope without disabling server capabilities
or reducing test coverage. The pre-change audit and timing evidence are in
`docs/development/build-time-audit-2026-10-25/`.
