//! Shared-table call into the versioned equality-update body.

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{ids::VersionId, models::UserId, TableType, TransactionId};
use kalamdb_transactions::{StagedMutation, TransactionOverlay};

use super::SharedTableProvider;
use crate::error::KalamDbError;

impl SharedTableProvider {
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
        crate::versioned::equality::plan_literal_equality_updates(
            &self.core,
            &self.store,
            &self.vector_columns,
            TableType::Shared,
            None,
            transaction_id,
            user_id,
            snapshot_commit_seq,
            overlay,
            seek_column,
            seek_value,
            assignments,
        )
        .await
    }
}
