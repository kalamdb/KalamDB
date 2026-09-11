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
mod prepared_execution_statement;

pub use prepared_execution_statement::PreparedExecutionStatement;
pub mod request_transaction_state;
mod sql_executor;
mod transaction_batch_insert;

use std::sync::Arc;

use datafusion::execution::context::SessionState;
pub use datafusion::scalar::ScalarValue;

use crate::sql::{
    executor::handler_registry::HandlerRegistry,
    plan_cache::{PlanCacheKey, SqlCacheRegistry},
};

/// Public facade for SQL execution routing.
pub struct SqlExecutor {
    app_context:              Arc<crate::app_context::AppContext>,
    handler_registry:         Arc<HandlerRegistry>,
    sql_cache_registry:       Arc<SqlCacheRegistry>,
    prepared_statement_cache: moka::sync::Cache<(PlanCacheKey, bool), PreparedExecutionStatement>,
    point_read_session_cache: moka::sync::Cache<
        point_read_session_cache_key::PointReadSessionCacheKey,
        Arc<SessionState>,
    >,
    #[cfg(test)]
    point_get_fast_path_hits: std::sync::atomic::AtomicU64,
}
