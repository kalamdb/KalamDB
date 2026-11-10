//! One engine over the moved user and shared provider bodies.
//!
//! The scan, DML, and hot-path functions still live in `engine/user` and
//! `engine/shared`. This type calls those bodies. Stream TTL stays on the
//! stream provider.

use datafusion::{logical_expr::Expr, scalar::ScalarValue};
use kalamdb_commons::{ids::VersionId, models::UserId, TransactionId};
use kalamdb_store::IndexRole;
use kalamdb_transactions::{StagedMutation, TransactionOverlay};

use crate::{
    error::KalamDbError, shared_tables::SharedTableProvider, user_tables::UserTableProvider,
    utils::TableRowDml,
};

pub(crate) enum VersionedTableEngine {
    User(UserTableProvider),
    Shared(SharedTableProvider),
}

impl VersionedTableEngine {
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

/// Whether a planned index seek is the authoritative hot path.
///
/// `since_seq` always reads the complete hot-plus-cold range. A deleted-row
/// query may use the primary-key index, which retains tombstones, and must
/// not use a secondary scalar index that omits them.
pub(crate) fn use_index_plan(
    role: IndexRole,
    keep_deleted: bool,
    since_seq: Option<VersionId>,
) -> bool {
    if since_seq.is_some() {
        return false;
    }
    !(keep_deleted && role == IndexRole::SecondaryScalar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_and_history_scans_use_the_authoritative_path() {
        let since = VersionId::try_from_i64(4).expect("version");
        assert!(
            !use_index_plan(IndexRole::SecondaryScalar, true, None),
            "_deleted = true must not use a secondary scalar index"
        );
        assert!(
            !use_index_plan(IndexRole::SecondaryScalar, false, Some(since)),
            "since_seq must not use a secondary scalar index"
        );
        assert!(
            !use_index_plan(IndexRole::PrimaryKey, false, Some(since)),
            "since_seq must scan the complete hot-plus-cold path"
        );
        assert!(use_index_plan(IndexRole::PrimaryKey, true, None));
        assert!(use_index_plan(IndexRole::SecondaryScalar, false, None));
    }
}
