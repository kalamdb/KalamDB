//! Stream table provider implementation with RLS + TTL
//!
//! This module provides StreamTableProvider implementing BaseTableProvider<StreamTableRowId,
//! StreamTableRow> for ephemeral event streams with Row-Level Security and TTL-based eviction.
//!
//! **Key Features**:
//! - Direct fields (no wrapper layer)
//! - Shared core via Arc<TableProviderCore>
//! - No handlers - all DML logic inline
//! - RLS via user_id parameter in DML methods
//! - Commit log-backed storage (append-only, no Parquet)
//! - TTL-based eviction in scan operations

use std::{
    collections::HashSet,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    catalog::Session,
    common::DFSchema,
    datasource::TableProvider,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{dml::InsertOp, Expr, TableProviderFilterPushDown},
    physical_expr::PhysicalExpr,
    physical_plan::ExecutionPlan,
    scalar::ScalarValue,
};
// Arrow <-> JSON helpers
use kalamdb_commons::models::rows::Row;
use kalamdb_commons::{
    conversions::arrow_json_conversion::coerce_rows,
    ids::{StreamTableRowId, VersionId},
    models::UserId,
    websocket::ChangeNotification,
};
use kalamdb_datafusion_sources::{
    exec::{
        finalize_deferred_batch, DeferredBatchExec, DeferredBatchOutput, DeferredBatchSource,
        DeferredScanDiagnostics,
    },
    provider::{
        combined_filter, merged_projection_scan_descriptor, pushdown_results_for_filters,
        remap_projection_indices, FilterCapability, ScanDescriptor, SourceProvider,
    },
};
use kalamdb_session_datafusion::{check_user_table_write_access, session_error_to_datafusion};
use tracing::Instrument;

use crate::{
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    stream_tables::{StreamTableRow, StreamTableStore},
    utils::{
        base::{
            extract_seq_bounds_from_filter, scan_diagnostics_enabled, BaseTableProvider,
            TableProviderCore,
        },
        row_utils::extract_user_context,
    },
};

/// Stream table provider with RLS and TTL filtering
///
/// **Architecture**:
/// - Stateless provider (user context passed per-operation)
/// - Direct fields (no wrapper layer)
/// - Shared core via Arc<TableProviderCore> (holds schema, pk_name, column_defaults,
///   non_null_columns)
/// - RLS enforced via user_id parameter
/// - HOT-ONLY storage (ephemeral data, no Parquet)
/// - TTL-based eviction
pub struct StreamTableProvider {
    /// Shared core (services, schema, pk_name, column_defaults, non_null_columns)
    core: Arc<TableProviderCore>,

    /// StreamTableStore for DML operations (commit log-backed)
    store: Arc<StreamTableStore>,

    /// TTL in seconds (optional)
    ttl_seconds: Option<u64>,
}

include!("scan_source.rs");
include!("operations.rs");
include!("sql.rs");
