use std::sync::Arc;

use arrow::{array::RecordBatch, datatypes::SchemaRef};
use kalamdb_commons::{
    conversions::arrow_json_conversion::json_rows_to_arrow_batch,
    models::rows::Row,
    MemoryClass, MemoryLedger, Reservation,
};

use super::query_stream::QueryBatchStream;

/// Result type for SQL execution
#[derive(Debug)]
pub enum ExecutionResult {
    /// Generic success message (DDL operations, user management, etc.)
    Success { message: String },
    /// Query results (SELECT, SHOW, DESCRIBE)
    Rows {
        batches: Vec<RecordBatch>,
        row_count: usize,
        /// Optional schema for when batches is empty (0 rows returned)
        /// This preserves column information for the UI even when no data
        schema: Option<SchemaRef>,
    },
    /// Read-only SELECT stream. Wire encodes from the stream. HTTP and other
    /// complete-result callers use [`Self::into_materialized_rows`].
    StreamingRows { stream: QueryBatchStream },
    /// Cached PK point-get rows that skip Arrow on the HTTP JSON path.
    ///
    /// HTTP `/v1/api/sql` must serialize this via `rows_to_json_arrays`, not
    /// `into_arrow_rows()`. Converting to [`ExecutionResult::Rows`] first
    /// rebuilds a RecordBatch and is the 2026-09-11 bake-off regression
    /// (16.88s / p50 245µs with Arrow vs 15.30s / p50 220µs without).
    /// pgwire, live queries, and cluster forwarding still call
    /// [`Self::into_arrow_rows`].
    ScalarRows {
        rows: Vec<Row>,
        row_count: usize,
        schema: SchemaRef,
    },
    /// INSERT result
    Inserted { rows_affected: usize },
    /// UPDATE result
    Updated { rows_affected: usize },
    /// DELETE result
    Deleted { rows_affected: usize },
    /// FLUSH result
    Flushed {
        tables: Vec<String>,
        bytes_written: u64,
    },
    /// Subscription metadata (for SUBSCRIBE/LIVE SELECT commands)
    Subscription {
        subscription_id: String,
        channel: String,
        select_query: String,
    },
    /// Job killed result
    JobKilled { job_id: String, status: String },
}

impl Clone for ExecutionResult {
    fn clone(&self) -> Self {
        match self {
            Self::Success { message } => Self::Success {
                message: message.clone(),
            },
            Self::Rows {
                batches,
                row_count,
                schema,
            } => Self::Rows {
                batches: batches.clone(),
                row_count: *row_count,
                schema: schema.clone(),
            },
            Self::StreamingRows { .. } => {
                panic!("StreamingRows cannot be cloned; materialize or consume the stream once")
            },
            Self::ScalarRows {
                rows,
                row_count,
                schema,
            } => Self::ScalarRows {
                rows: rows.clone(),
                row_count: *row_count,
                schema: schema.clone(),
            },
            Self::Inserted { rows_affected } => Self::Inserted {
                rows_affected: *rows_affected,
            },
            Self::Updated { rows_affected } => Self::Updated {
                rows_affected: *rows_affected,
            },
            Self::Deleted { rows_affected } => Self::Deleted {
                rows_affected: *rows_affected,
            },
            Self::Flushed {
                tables,
                bytes_written,
            } => Self::Flushed {
                tables: tables.clone(),
                bytes_written: *bytes_written,
            },
            Self::Subscription {
                subscription_id,
                channel,
                select_query,
            } => Self::Subscription {
                subscription_id: subscription_id.clone(),
                channel: channel.clone(),
                select_query: select_query.clone(),
            },
            Self::JobKilled { job_id, status } => Self::JobKilled {
                job_id: job_id.clone(),
                status: status.clone(),
            },
        }
    }
}

impl ExecutionResult {
    /// Get row_count or rows_affected for response serialization
    pub fn affected_rows(&self) -> usize {
        match self {
            ExecutionResult::Rows { row_count, .. } => *row_count,
            ExecutionResult::StreamingRows { .. } => 0,
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

    /// True when wire or HTTP should pull batches from a live stream.
    pub fn is_streaming_rows(&self) -> bool {
        matches!(self, ExecutionResult::StreamingRows { .. })
    }

    /// Drain [`ExecutionResult::StreamingRows`] into [`ExecutionResult::Rows`].
    pub async fn into_materialized_rows(self) -> Result<Self, String> {
        match self {
            ExecutionResult::StreamingRows { stream } => {
                let (batches, schema, row_count) = stream.collect_with_row_count().await?;
                Ok(ExecutionResult::Rows {
                    batches,
                    row_count,
                    schema: Some(schema),
                })
            },
            other => Ok(other),
        }
    }

    /// Like [`Self::into_materialized_rows`], charging [`MemoryClass::ScanBatch`].
    ///
    /// Drop the reservation after the caller finishes with the batches so
    /// logical live bytes return to baseline.
    pub async fn into_materialized_rows_accounted(
        self,
        ledger: &Arc<MemoryLedger>,
    ) -> Result<(Self, Reservation), String> {
        match self {
            ExecutionResult::StreamingRows { stream } => {
                let (batches, schema, reservation) = stream.collect_accounted(ledger).await?;
                let row_count = batches.iter().map(RecordBatch::num_rows).sum();
                Ok((
                    ExecutionResult::Rows {
                        batches,
                        row_count,
                        schema: Some(schema),
                    },
                    reservation,
                ))
            },
            other => Ok((other, Reservation::adopt(ledger, MemoryClass::ScanBatch, 0))),
        }
    }

    /// Convert [`ExecutionResult::ScalarRows`] into Arrow [`ExecutionResult::Rows`].
    ///
    /// Do not use this on the HTTP cached point-get path. Wire, live, cluster,
    /// and functions adapters need Arrow; HTTP JSON does not.
    ///
    /// Streaming results must use [`Self::into_materialized_rows`] first.
    pub fn into_arrow_rows(self) -> Result<Self, String> {
        match self {
            ExecutionResult::StreamingRows { .. } => Err(
                "internal: streaming rows require materialization or the async encoder".to_string(),
            ),
            ExecutionResult::ScalarRows {
                rows,
                row_count,
                schema,
            } => {
                let _span = kalamdb_observability::kdb_info_span_entered!(
                    "sql.scalar_to_arrow",
                    row_count = row_count
                );
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
