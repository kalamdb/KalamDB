use std::sync::Arc;

use kalamdb_commons::{models::TableId, schemas::TableType};
use kalamdb_sql::classifier::SqlStatement;
use sqlparser::ast::Statement;

/// Execution metadata prepared once and reused by cache hits and wire portals.
///
/// Parsed trees are immutable and shared so cloning metadata does not recursively
/// allocate a copy of every SQL expression. Bind values stay request-local.
#[derive(Debug, Clone)]
pub struct PreparedExecutionStatement {
    pub sql:                  String,
    pub table_id:             Option<TableId>,
    pub table_type:           Option<TableType>,
    pub classified_statement: Option<Arc<SqlStatement>>,
    /// Precomputed at prepare time; checked with a single bool on the execution hot path.
    pub track_slow_query:     bool,
    /// DML sqlparser AST parsed once at prepare; reused by literal insert, batch insert,
    /// and system.migrations handlers so execute does not re-parse.
    pub parsed_dml:           Option<Arc<Statement>>,
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
            classified_statement: classified_statement.map(Arc::new),
            track_slow_query,
            parsed_dml: parsed_dml.map(Arc::new),
        }
    }
}
