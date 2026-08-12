//! SQL executor orchestrator (Phase 9 foundations)
//!
//! Minimal dispatcher that keeps the public `SqlExecutor` API compiling
//! while provider-based handlers are migrated. Behaviour is intentionally
//! limited for now; most statements return a structured error until
//! handler implementations are in place.

pub mod default_ordering;
pub mod handler_adapter;
pub mod handler_registry;
pub mod handlers;
pub mod parameter_binding;
mod point_read_session_cache_key;
pub mod request_transaction_state;
mod sql_executor;
mod transaction_batch_insert;

use std::sync::Arc;

use datafusion::execution::context::SessionState;
pub use datafusion::scalar::ScalarValue;
use kalamdb_commons::{models::TableId, schemas::TableType};
use kalamdb_sql::classifier::SqlStatement;
use sqlparser::ast::Statement;

use crate::sql::{
    executor::handler_registry::HandlerRegistry,
    plan_cache::{PlanCacheKey, SqlCacheRegistry},
};

/// Public facade for SQL execution routing.
pub struct SqlExecutor {
    app_context: Arc<crate::app_context::AppContext>,
    handler_registry: Arc<HandlerRegistry>,
    sql_cache_registry: Arc<SqlCacheRegistry>,
    prepared_statement_cache: moka::sync::Cache<(PlanCacheKey, bool), PreparedExecutionStatement>,
    point_read_session_cache: moka::sync::Cache<
        point_read_session_cache_key::PointReadSessionCacheKey,
        Arc<SessionState>,
    >,
    #[cfg(test)]
    point_get_fast_path_hits: std::sync::atomic::AtomicU64,
}

/// Optional execution context prepared by upstream callers (e.g. API layer).
#[derive(Debug, Clone)]
pub struct PreparedExecutionStatement {
    pub sql: String,
    pub table_id: Option<TableId>,
    pub table_type: Option<TableType>,
    pub classified_statement: Option<SqlStatement>,
    /// Precomputed at prepare time; checked with a single bool on the execution hot path.
    pub track_slow_query: bool,
    /// DML sqlparser AST parsed once at prepare; reused by literal insert, batch insert,
    /// and system.migrations handlers so execute does not re-parse.
    pub parsed_dml: Option<Statement>,
}

impl PreparedExecutionStatement {
    /// `track_slow_query` must come from the already-classified statement kind
    /// (see `SqlStatement::is_slow_query_trackable`) — no SQL re-parsing.
    pub fn new(
        sql: String,
        table_id: Option<TableId>,
        table_type: Option<TableType>,
        classified_statement: Option<SqlStatement>,
        track_slow_query: bool,
        parsed_dml: Option<Statement>,
    ) -> Self {
        Self {
            sql,
            table_id,
            table_type,
            classified_statement,
            track_slow_query,
            parsed_dml,
        }
    }
}
