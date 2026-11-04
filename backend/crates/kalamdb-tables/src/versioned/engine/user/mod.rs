//! User table provider implementation with RLS
//!
//! This module provides UserTableProvider implementing BaseTableProvider<UserTableRowId,
//! UserTableRow> with Row-Level Security (RLS) enforced via user_id parameter.
//!
//! **Key Features**:
//! - Direct fields (no UserTableShared wrapper)
//! - Shared core via Arc<TableProviderCore>
//! - No handlers - all DML logic inline
//! - RLS via user_id parameter in DML methods
//! - PK index for row lookup

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
    logical_expr::{dml::InsertOp, Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    ids::{SeqId, UserTableRowId, VersionId},
    models::{rows::Row, UserId},
    websocket::ChangeNotification,
    StorageKey, TableType,
};
use kalamdb_datafusion_sources::dml::PreparedUpdateAssignments;
use kalamdb_datafusion_sources::{
    exec::{pk_bucket_key_from_row, resolve_latest_kvs_from_cold_batch, DeferredScanDiagnostics},
    provider::{
        merged_projection_scan_descriptor, mvcc_filter_capability, FilterCapability,
        ScanDescriptor, SourceProvider,
    },
};
use kalamdb_session::can_read_all_users;
use kalamdb_session_datafusion::{
    check_user_table_access, check_user_table_write_access, session_error_to_datafusion,
};
use kalamdb_store::EntityStore;
use kalamdb_transactions::extract_transaction_query_context;
use kalamdb_vector::{new_indexed_user_vector_hot_store, UserVectorHotOpId, UserVectorHotStore};
use tracing::Instrument;

use crate::{
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    manifest::manifest_helpers::{ensure_manifest_ready, load_row_from_parquet_by_seq},
    user_tables::{UserTableIndexedStore, UserTableRow},
    utils::{
        base::{self, BaseTableProvider, DeferredMvccScanProvider, TableProviderCore},
        row_utils::extract_user_context,
    },
    versioned::pk_index::{new_user_pk_index, PkPrefix, VersionedPkIndex},
};

/// User table provider with RLS
///
/// **Architecture**:
/// - Stateless provider (user context passed per-operation)
/// - Direct fields (no wrapper layer)
/// - Shared core via Arc<TableProviderCore> (holds schema, pk_name, column_defaults,
///   non_null_columns)
/// - RLS enforced via user_id parameter
/// - PK Index for efficient row lookup (Phase 14)
#[derive(Clone)]
pub(crate) struct UserTableProvider {
    /// Shared core (services, schema, pk_name, column_defaults, non_null_columns)
    core: Arc<TableProviderCore>,

    /// IndexedEntityStore with PK index for DML operations (public for flush jobs)
    pub(crate) store: Arc<UserTableIndexedStore>,

    /// PK index for efficient lookups
    pk_index: VersionedPkIndex,

    /// Embedding columns tracked by vector hot staging: (column_name, dimensions).
    vector_columns: Vec<(String, u32)>,

    /// Cached vector staging stores keyed by embedding column name.
    vector_stores: HashMap<String, Arc<UserVectorHotStore>>,
}

include!("storage.rs");
include!("scan.rs");
include!("dml.rs");
include!("sql.rs");
