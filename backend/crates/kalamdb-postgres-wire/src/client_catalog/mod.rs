//! PostgreSQL wire client catalog compatibility.
//!
//! Owns protocol-facing compat that GUI clients (Tabularis, DBeaver, Beekeeper,
//! pgAdmin) expect. DataFusion `TableProvider` implementations for populated
//! `pg_catalog` / `information_schema` views remain in `kalamdb-views` (shared
//! with HTTP), but **empty-table schemas and `SET` handling** are defined here
//! so wire coverage stays in one place.
//!
//! Compared with [datafusion-postgres `datafusion-pg-catalog`](https://github.com/datafusion-contrib/datafusion-postgres):
//! we ship empty shims for the P0/P1 probe tables rather than full PG17 feather dumps.

pub mod empty_tables;
pub mod postgres_set;

pub use empty_tables::{empty_pg_catalog_table_names, EMPTY_PG_CATALOG_TABLE_COUNT};
pub use postgres_set::{
    classify_postgres_set, normalize_search_path_schema, PostgresSetAction,
};
