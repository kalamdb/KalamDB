//! Implementation of SharedDataApplier for provider persistence
//!
//! This module provides the concrete implementation of `kalamdb_raft::SharedDataApplier`
//! that persists shared table data to the actual providers (RocksDB-backed stores).
//!
//! Called by SharedDataStateMachine after Raft consensus on all nodes.

use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_commons::{
    ids::VersionId,
    models::{rows::Row, TransactionId, UserId},
    TableId,
};
use kalamdb_raft::{GroupId, RaftError, SharedDataApplier, TransactionApplyResult};
use kalamdb_transactions::StagedMutation;

use crate::{app_context::AppContext, applier::executor::CommandExecutorImpl};

/// SharedDataApplier implementation using Unified Command Executor
///
/// This is called by the Raft state machine when applying committed commands.
/// It delegates to `CommandExecutorImpl::dml()` for actual logic.
pub struct ProviderSharedDataApplier {
    executor: CommandExecutorImpl,
}

impl ProviderSharedDataApplier {
    /// Create a new ProviderSharedDataApplier
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            executor: CommandExecutorImpl::new(app_context),
        }
    }
}

#[async_trait]
impl SharedDataApplier for ProviderSharedDataApplier {
    fn shared_shard_id(&self, table_id: &TableId) -> Result<u32, RaftError> {
        match self
            .executor
            .app_context()
            .shared_group_id(table_id)
            .map_err(|error| RaftError::provider(error.to_string()))?
        {
            GroupId::DataSharedShard(shard) => Ok(shard),
            _ => Err(RaftError::InvalidState("Expected shared owner".into())),
        }
    }

    async fn insert(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        rows: &[Row],
        encoded_fields: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<usize, RaftError> {
        let rows = {
            let _decode_span = kalamdb_observability::kdb_info_span_entered!("raft.decode_rows");
            crate::applier::ordinal_dml::decode_insert_rows(
                self.executor.app_context(),
                table_id,
                rows,
                encoded_fields,
            )
            .map_err(RaftError::provider)?
        };

        log::debug!("ProviderSharedDataApplier: Inserting into {} ({} rows)", table_id, rows.len());

        self.executor
            .dml()
            .insert_shared_data_with_versions(table_id, actor_user_id, &rows, versions)
            .await
            .map_err(|e| RaftError::provider(e.to_string()))
    }

    async fn update(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        updates: &[Row],
        pk_values: Option<&[String]>,
        filter: Option<&str>,
        versions: &[VersionId],
    ) -> Result<usize, RaftError> {
        log::debug!("ProviderSharedDataApplier: Updating {} ({} rows)", table_id, updates.len());

        self.executor
            .dml()
            .update_shared_data_with_versions(
                table_id,
                actor_user_id,
                updates,
                pk_values,
                filter,
                versions,
            )
            .await
            .map_err(|e| RaftError::provider(e.to_string()))
    }

    async fn delete(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        pk_values: Option<&[String]>,
        versions: &[VersionId],
    ) -> Result<usize, RaftError> {
        log::debug!("ProviderSharedDataApplier: Deleting from {}", table_id);

        self.executor
            .dml()
            .delete_shared_data_with_versions(table_id, actor_user_id, pk_values, versions)
            .await
            .map_err(|e| RaftError::provider(e.to_string()))
    }

    async fn apply_transaction_batch(
        &self,
        transaction_id: &TransactionId,
        mutations: &[StagedMutation],
        versions: &[VersionId],
    ) -> Result<TransactionApplyResult, RaftError> {
        self.executor
            .dml()
            .apply_shared_transaction_batch_with_versions(transaction_id, mutations, versions)
            .await
            .map_err(|e| RaftError::provider(e.to_string()))
    }
}
