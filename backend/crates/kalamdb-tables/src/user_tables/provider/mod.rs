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
//! - SessionState extraction for scan_rows()
//! - PK Index for efficient row lookup (Phase 14)

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
    conversions::{
        arrow_json_conversion::{coerce_rows, coerce_updates},
        parse_string_as_scalar,
    },
    ids::{SeqId, UserTableRowId, VersionId},
    models::{rows::Row, OperationKind, UserId},
    websocket::ChangeNotification,
    StorageKey, TableType,
};
use kalamdb_datafusion_sources::{
    exec::{
        pk_bucket_key_from_row, resolve_latest_kvs_from_cold_batch, DeferredScanDiagnostics,
        PkBucketKey, VersionedRow,
    },
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
use kalamdb_transactions::{extract_transaction_query_context, StagedMutation};
use kalamdb_vector::{new_indexed_user_vector_hot_store, UserVectorHotOpId, UserVectorHotStore};
use tracing::Instrument;

use crate::{
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    manifest::manifest_helpers::{ensure_manifest_ready, load_row_from_parquet_by_seq},
    user_tables::{UserTableIndexedStore, UserTablePkIndex, UserTableRow},
    utils::{
        base::{self, BaseTableProvider, DeferredMvccScanProvider, TableProviderCore},
        prepared_update_assignments::PreparedUpdateAssignments,
        row_utils::extract_user_context,
    },
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
pub struct UserTableProvider {
    /// Shared core (services, schema, pk_name, column_defaults, non_null_columns)
    core: Arc<TableProviderCore>,

    /// IndexedEntityStore with PK index for DML operations (public for flush jobs)
    pub(crate) store: Arc<UserTableIndexedStore>,

    /// PK index for efficient lookups
    pk_index: UserTablePkIndex,

    /// Embedding columns tracked by vector hot staging: (column_name, dimensions).
    vector_columns: Vec<(String, u32)>,

    /// Cached vector staging stores keyed by embedding column name.
    vector_stores: HashMap<String, Arc<UserVectorHotStore>>,
}

struct UserMvccRow(UserTableRow);

impl VersionedRow for UserMvccRow {
    fn version(&self) -> VersionId {
        self.0._version
    }

    fn deleted(&self) -> bool {
        self.0._deleted
    }

    fn pk_value(&self, pk_name: &str) -> Option<String> {
        match self.pk_bucket_key(pk_name) {
            PkBucketKey::Version(_) => None,
            key => Some(key.to_string()),
        }
    }

    fn pk_bucket_key(&self, pk_name: &str) -> PkBucketKey {
        pk_bucket_key_from_row(&self.0.fields, pk_name, self.0._version)
    }
}

fn version_from_commit_seq(commit_seq: u64, ordinal: u32) -> Result<VersionId, KalamDbError> {
    crate::utils::base::statement_row_version(commit_seq, ordinal)
}

include!("storage.rs");
include!("scan.rs");
include!("dml.rs");
include!("sql.rs");
