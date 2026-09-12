use arrow::{array::RecordBatch, datatypes::SchemaRef};
use kalamdb_commons::{
    conversions::arrow_json_conversion::json_rows_to_arrow_batch, models::rows::Row,
};

/// Result type for SQL execution
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    /// Generic success message (DDL operations, user management, etc.)
    Success { message: String },
    /// Query results (SELECT, SHOW, DESCRIBE)
    Rows {
        batches:   Vec<RecordBatch>,
        row_count: usize,
        /// Optional schema for when batches is empty (0 rows returned)
        /// This preserves column information for the UI even when no data
        schema:    Option<SchemaRef>,
    },
    /// Cached PK point-get rows that skip Arrow on the HTTP JSON path.
    ///
    /// HTTP `/v1/api/sql` must serialize this via `rows_to_json_arrays`, not
    /// `into_arrow_rows()`. Converting to [`ExecutionResult::Rows`] first
    /// rebuilds a RecordBatch and is the 2026-09-11 bake-off regression
    /// (16.88s / p50 245µs with Arrow vs 15.30s / p50 220µs without).
    /// pgwire, live queries, and cluster forwarding still call
    /// [`Self::into_arrow_rows`].
    ScalarRows {
        rows:      Vec<Row>,
        row_count: usize,
        schema:    SchemaRef,
    },
    /// INSERT result
    Inserted { rows_affected: usize },
    /// UPDATE result
    Updated { rows_affected: usize },
    /// DELETE result
    Deleted { rows_affected: usize },
    /// FLUSH result
    Flushed {
        tables:        Vec<String>,
        bytes_written: u64,
    },
    /// Subscription metadata (for SUBSCRIBE/LIVE SELECT commands)
    Subscription {
        subscription_id: String,
        channel:         String,
        select_query:    String,
    },
    /// Job killed result
    JobKilled { job_id: String, status: String },
}

impl ExecutionResult {
    /// Get row_count or rows_affected for response serialization
    pub fn affected_rows(&self) -> usize {
        match self {
            ExecutionResult::Rows { row_count, .. } => *row_count,
            ExecutionResult::ScalarRows { row_count, .. } => *row_count,
            ExecutionResult::Inserted { rows_affected } => *rows_affected,
            ExecutionResult::Updated { rows_affected } => *rows_affected,
            ExecutionResult::Deleted { rows_affected } => *rows_affected,
            ExecutionResult::Flushed { tables, .. } => tables.len(),
            ExecutionResult::Success { .. } => 1,
            ExecutionResult::Subscription { .. } => 1,
            ExecutionResult::JobKilled { .. } => 1,
        }
    }

    /// True when this result skipped Arrow on the HTTP JSON path.
    pub fn is_scalar_rows(&self) -> bool {
        matches!(self, ExecutionResult::ScalarRows { .. })
    }

    /// Convert [`ExecutionResult::ScalarRows`] into Arrow [`ExecutionResult::Rows`].
    ///
    /// Do not use this on the HTTP cached point-get path. Wire, live, cluster,
    /// and functions adapters need Arrow; HTTP JSON does not.
    pub fn into_arrow_rows(self) -> Result<Self, String> {
        match self {
            ExecutionResult::ScalarRows {
                rows,
                row_count,
                schema,
            } => {
                let batch = if rows.is_empty() {
                    RecordBatch::new_empty(std::sync::Arc::clone(&schema))
                } else {
                    json_rows_to_arrow_batch(&schema, rows)?
                };
                Ok(ExecutionResult::Rows {
                    batches: vec![batch],
                    row_count,
                    schema: Some(schema),
                })
            },
            other => Ok(other),
        }
    }
}
