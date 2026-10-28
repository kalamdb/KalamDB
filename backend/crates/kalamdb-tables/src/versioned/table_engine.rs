//! One engine over the moved user and shared provider bodies.
//!
//! The scan, DML, and hot-path functions still live in `engine/user` and
//! `engine/shared`. This type calls those bodies. Stream TTL stays on the
//! stream provider.

use std::sync::Arc;

use datafusion::{logical_expr::Expr, scalar::ScalarValue};
use kalamdb_commons::{
    ids::{ScopedRowKey, VersionId},
    models::{rows::VersionedTableRow, UserId},
    TransactionId,
};
use kalamdb_serialization::StorageSchema;
use kalamdb_store::storage_trait::Result as StoreResult;
use kalamdb_transactions::{StagedMutation, TransactionOverlay};

use crate::{
    error::KalamDbError, shared_tables::SharedTableProvider, user_tables::UserTableProvider,
    utils::TableRowDml, versioned::VersionedRowCodec,
};

pub enum VersionedTableEngine {
    User(UserTableProvider),
    Shared(SharedTableProvider),
}

impl VersionedTableEngine {
    pub fn encode_row(
        schema: Arc<StorageSchema>,
        key: &ScopedRowKey,
        row: &VersionedTableRow,
    ) -> StoreResult<Vec<u8>> {
        VersionedRowCodec::new(schema).encode(key, row)
    }

    pub fn decode_row(
        schema: Arc<StorageSchema>,
        key: &ScopedRowKey,
        bytes: &[u8],
    ) -> StoreResult<VersionedTableRow> {
        VersionedRowCodec::new(schema).decode(key, bytes)
    }

    pub async fn columnar_hot_scan(
        &self,
        user_id: Option<&UserId>,
        snapshot: Option<VersionId>,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(datafusion::arrow::record_batch::RecordBatch, usize)>, KalamDbError> {
        match self {
            Self::Shared(provider) => {
                provider
                    .columnar_hot_scan(snapshot, projection, filter, limit, keep_deleted)
                    .await
            },
            Self::User(provider) => {
                let Some(user_id) = user_id else {
                    return Ok(None);
                };
                provider
                    .columnar_hot_scan(user_id, snapshot, projection, filter, limit, keep_deleted)
                    .await
            },
        }
    }

    pub async fn sum_hot_covering_integers(
        &self,
        user_id: Option<&UserId>,
        filters: &[Expr],
        value_column: &str,
    ) -> Result<Option<(u64, i128)>, KalamDbError> {
        match self {
            Self::Shared(provider) => {
                provider.sum_hot_covering_integers(filters, value_column).await
            },
            Self::User(provider) => {
                let Some(user_id) = user_id else {
                    return Ok(None);
                };
                provider.sum_hot_covering_integers(user_id, filters, value_column).await
            },
        }
    }

    pub async fn plan_literal_equality_updates(
        &self,
        transaction_id: &TransactionId,
        user_id: &UserId,
        snapshot_commit_seq: Option<VersionId>,
        overlay: Option<&TransactionOverlay>,
        seek_column: &str,
        seek_value: &ScalarValue,
        assignments: &[(String, ScalarValue)],
    ) -> Result<Option<Vec<StagedMutation>>, KalamDbError> {
        match self {
            Self::Shared(provider) => {
                provider
                    .plan_literal_equality_updates(
                        transaction_id,
                        user_id,
                        snapshot_commit_seq,
                        overlay,
                        seek_column,
                        seek_value,
                        assignments,
                    )
                    .await
            },
            Self::User(provider) => {
                provider
                    .plan_literal_equality_updates(
                        transaction_id,
                        user_id,
                        snapshot_commit_seq,
                        overlay,
                        seek_column,
                        seek_value,
                        assignments,
                    )
                    .await
            },
        }
    }

    /// The user or shared provider that implements DataFusion and row DML.
    pub fn as_row_dml(&self) -> &dyn TableRowDml {
        match self {
            Self::User(provider) => provider,
            Self::Shared(provider) => provider,
        }
    }
}
