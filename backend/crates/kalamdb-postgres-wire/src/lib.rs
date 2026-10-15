//! PostgreSQL wire protocol adapter for KalamDB.
//!
//! This crate owns the connection: startup, extended-query messages, and row
//! encoding. SQL behavior is `SqlExecutor` on the shared DataFusion session, so
//! the same statements work on `/api/sql`.

pub mod connection;
pub mod handlers;
pub mod listener;
pub mod params;
mod pg_types;
pub mod ports;
pub mod query;
pub mod row_encoder;
pub mod server;
pub mod sql_exec;
pub mod startup;
pub mod statement;
pub mod tx_control;

pub use listener::{format_startup_log_segment, PostgresWireListener, PostgresWireRuntimeDeps};
pub use ports::{http_port_conflict_message, rpc_port_conflict_message};
pub use startup::{resolve_wire_startup_schema, WIRE_LOGICAL_DATABASE};
