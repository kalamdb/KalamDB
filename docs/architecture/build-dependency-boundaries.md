# Build dependency boundaries

The CLI and SQL dialect must not depend on the system-table providers, RocksDB
store, or DataFusion execution engine. They share `KalamParser` and its AST with
the server. Arrow and `datafusion-common` remain available for shared schema and
scalar representations.

`kalamdb-commons::models` owns `StorageMode`, `StorageType`, `VectorMetric`, and
the shared identifier types. `SystemTable` is also exported by commons. System
providers retain their existing public re-exports so callers use the same types.

Table-provider ownership stays on the crate that owns the data.
`kalamdb-tables` exposes `KalamTableProvider` for user, shared, and stream
tables. `kalamdb-system` exposes `KeyedTableProvider` for persisted system
stores. `kalamdb-views` exposes `ComputedCatalogProvider` for virtual
`system.*`, `pg_catalog`, and Kalam `information_schema` projections.
`kalamdb-transfer` does not depend on `kalamdb-core`. `kalamdb-publisher` does
not depend on `kalamdb-tables`. `kalamdb-jobs` owns job executors and installs
them into `AppContext`; `kalamdb-core` does not depend on `kalamdb-jobs`.
Short dashboard reads of `system.*` use the HTTP SQL fast path in
`backend/crates/kalamdb-api/src/http/sql/execute.rs` (`is_system_catalog_read`).
A join out of `system`, or any write, does not take that path.

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

`kalamdb-core` integration tests link as four binaries (`pg18`, `sql_semantics`,
`transactions`, `storage_ops`) instead of one executable per source file. Run a
single case with a name filter, for example
`cargo nextest run -p kalamdb-core prepared_metadata_cache`. Actix content
encoding is off; cookies, HTTP/2, and WebSocket stay enabled. DataFusion still
re-enables Parquet's default codecs, and both Rustls providers stay because
Reqwest uses ring while JWT, gRPC, and pgwire use AWS-LC.
