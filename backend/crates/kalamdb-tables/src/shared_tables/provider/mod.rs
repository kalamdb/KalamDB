//! Shared-table storage with FORCE row-level security at SQL and live boundaries.
//!
//! This module provides SharedTableProvider implementing BaseTableProvider<SharedTableRowId,
//! SharedTableRow>. Low-level storage remains principal-agnostic; public SQL, typed DML, and live
//! paths bind and enforce RLS before returning or mutating rows.

#[path = "../shared_table_authorization.rs"]
mod shared_table_authorization;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    catalog::Session,
    datasource::TableProvider,
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    logical_expr::{dml::InsertOp, Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::{
        arrow_json_conversion::{coerce_rows, coerce_updates},
        parse_string_as_scalar,
    },
    ids::{SeqId, SharedTableRowId, VersionId},
    models::{rows::Row, OperationKind, UserId},
    websocket::ChangeNotification,
    NotLeaderError, PolicyCommand, TableId, TableType,
};
use kalamdb_datafusion_sources::{
    exec::{pk_bucket_key_from_row, DeferredScanDiagnostics},
    provider::{
        merged_projection_scan_descriptor, mvcc_filter_capability, FilterCapability,
        ScanDescriptor, SourceProvider,
    },
};
use kalamdb_rls::BoundLiveAuthorization;
use kalamdb_session_datafusion::{
    check_shared_table_access, check_shared_table_write_access, session_error_to_datafusion,
    RlsCommandContext,
};
use kalamdb_store::EntityStore;
use kalamdb_transactions::{extract_transaction_query_context, StagedMutation};
use kalamdb_vector::{
    new_indexed_shared_vector_hot_store, SharedVectorHotOpId, SharedVectorHotStore,
};
use shared_table_authorization::SharedTableAuthorization;
use tracing::Instrument;

use crate::{
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    manifest::manifest_helpers::{ensure_manifest_ready, load_row_from_parquet_by_seq},
    shared_tables::{SharedTableIndexedStore, SharedTablePkIndex, SharedTableRow},
    utils::{
        base::{self, BaseTableProvider, DeferredMvccScanProvider, TableProviderCore},
        prepared_update_assignments::PreparedUpdateAssignments,
        row_utils::{extract_full_user_context, extract_user_context},
    },
};

/// Shared table provider without RLS
///
/// **Architecture**:
/// - Stateless provider (user context passed but ignored)
/// - Direct fields (no wrapper layer)
/// - Shared core via Arc<TableProviderCore> (holds schema, pk_name, column_defaults,
///   non_null_columns, table_def)
/// - NO RLS - user_id parameter ignored in all operations
/// - Uses SharedTableIndexedStore for efficient PK lookups
#[derive(Clone)]
pub struct SharedTableProvider {
    /// Shared core (services, schema, pk_name, column_defaults, non_null_columns, table_def)
    core: Arc<TableProviderCore>,

    /// SharedTableIndexedStore for DML operations with PK index
    pub(crate) store: Arc<SharedTableIndexedStore>,

    /// PK index for efficient lookups
    pk_index: SharedTablePkIndex,

    /// Embedding columns tracked by vector hot staging: (column_name, dimensions).
    vector_columns: Vec<(String, u32)>,

    /// Cached vector staging stores keyed by embedding column name.
    vector_stores: HashMap<String, Arc<SharedVectorHotStore>>,

    /// Shared-table row-level security orchestration.
    pub(crate) authorization: SharedTableAuthorization,
}

fn version_from_commit_seq(commit_seq: u64, ordinal: u32) -> Result<VersionId, KalamDbError> {
    crate::utils::base::statement_row_version(commit_seq, ordinal)
}

include!("storage.rs");
include!("pk_maintenance.rs");
include!("scan.rs");
include!("dml.rs");
include!("sql.rs");
