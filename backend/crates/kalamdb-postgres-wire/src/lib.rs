//! PostgreSQL wire protocol adapter for KalamDB.
//!
//! Client catalog compatibility (`SET`, empty `pg_catalog` probe tables) lives in
//! [`client_catalog`]. Populated catalog providers stay in `kalamdb-views`.

pub mod client_catalog;
pub mod connection;
pub mod handlers;
pub mod listener;
pub mod params;
pub mod ports;
pub mod query;
pub mod row_encoder;
pub mod server;
pub mod sql_exec;
pub mod startup;
pub mod statement;
pub mod tx_control;

pub use client_catalog::{
    classify_postgres_set, empty_pg_catalog_table_names, normalize_search_path_schema,
    PostgresSetAction, EMPTY_PG_CATALOG_TABLE_COUNT,
};
pub use listener::{format_startup_log_segment, PostgresWireListener, PostgresWireRuntimeDeps};
pub use ports::{http_port_conflict_message, rpc_port_conflict_message};
pub use startup::{resolve_wire_startup_schema, WIRE_LOGICAL_DATABASE};
