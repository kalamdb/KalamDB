# Baselines: Unified Table Processing and Atomic Transaction Optimization

**Feature**: `039-unified-table-transactions`  
**Branch**: `feat/pg-dialect`  
**Profile**: `dev`  
**Hardware**: Apple M5 Pro, 15 cores, 24 GiB RAM, arm64, macOS 27.0.1  
**Recorded**: 2026-10-27, before provider edits

Pre-change rows are the comparison gate. Candidate rows are filled only after the change they measure. A blank candidate cell means that workload has not been re-run yet.

## Consolidation production lines

Counted by the quickstart script. A file's production lines stop at the first `#[cfg(test)]` module. File moves and deleted tests do not count as reduction.

| Root | Pre-change lines | Candidate lines |
|------|------------------|-----------------|
| `backend/crates/kalamdb-system/src/providers` | 13285 | |
| `backend/crates/kalamdb-system/src/macros.rs` | 434 | |
| `backend/crates/kalamdb-tables/src` | 18269 | |
| `backend/crates/kalamdb-flush/src/flush` | 1429 | |
| `backend/crates/kalamdb-core/src/applier/executor/dml.rs` | 1852 | |
| `backend/crates/kalamdb-jobs/src/executors/flush.rs` | 642 | |
| **Total** | **35911** | |

## Workload timings

Columns follow T001. `batch submissions` applies to commit workloads. Lookup and introspection rows leave it blank. Median and p99 use nearest rank over five matched `dev` runs, so p99 is the slowest of the five. Times are the nextest-reported test duration after the harness is already compiled, in seconds.

| Workload | Build profile | Hardware note | Median seconds | p99 seconds | Batch submissions |
|----------|---------------|---------------|----------------|-------------|-------------------|
| Pre-change `system.users` primary-key lookup (`test_create_and_get_user`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Pre-change `system.users` email index lookup (`test_get_active_oidc_invite_by_email_uses_email_index`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Pre-change `system.jobs` primary-key lookup (`test_create_and_get_job`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Pre-change namespace lookup (`test_create_and_get_namespace`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Pre-change catalog type lookup (`implicit_row_type_and_alias_have_catalog_relationships`) | dev | Apple M5 Pro, 24 GiB | 0.014 | 0.017 | |
| Pre-change `pg_catalog` namespaces, tables, and columns (`pg_catalog_shims_project_namespaces_tables_columns_and_database`, `kalamdb-core` `--test pg18`) | dev | Apple M5 Pro, 24 GiB | 0.080 | 0.112 | |
| Pre-change `information_schema.columns` introspection (`pg_catalog_shim_beekeeper_information_schema_column_query`, `kalamdb-core` `--test pg18`) | dev | Apple M5 Pro, 24 GiB | 0.085 | 0.115 | |
| Pre-change 25,000-row insert | dev | Apple M5 Pro, 24 GiB | | | |
| Pre-change 25,000-row update | dev | Apple M5 Pro, 24 GiB | | | |
| Pre-change 25,000-row delete | dev | Apple M5 Pro, 24 GiB | | | |
| Candidate system-catalog lookups and introspection | dev | Apple M5 Pro, 24 GiB | | | |
| Candidate 25,000-row insert, update, and delete | dev | Apple M5 Pro, 24 GiB | | | |

The 25,000-row rows stay empty until Gate 3. Candidate rows stay empty until the matching story is measured.
