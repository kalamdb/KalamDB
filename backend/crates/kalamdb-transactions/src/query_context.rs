use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use kalamdb_commons::{
    models::{rows::Row, OperationKind, TableId, TransactionId, UserId},
    TableType,
};

use crate::{
    access::{TransactionAccessError, TransactionAccessValidator},
    commit_sequence::SNAPSHOT_UNSET,
    overlay::TransactionOverlay,
    staged_mutation::StagedMutation,
};

/// Lightweight view trait exposed to query providers for transaction-local reads.
pub trait TransactionOverlayView: std::fmt::Debug + Send + Sync {
    fn overlay(&self) -> TransactionOverlay;

    fn overlay_for_table(&self, table_id: &TableId) -> Option<TransactionOverlay>;
}

pub trait TransactionMutationSink: std::fmt::Debug + Send + Sync {
    fn stage_mutation(
        &self,
        transaction_id: &TransactionId,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<UserId>,
        operation_kind: OperationKind,
        primary_key: String,
        row: Row,
        is_deleted: bool,
    ) -> Result<(), TransactionAccessError>;

    fn stage_batch(
        &self,
        transaction_id: &TransactionId,
        mutations: Vec<StagedMutation>,
    ) -> Result<(), TransactionAccessError> {
        for mutation in mutations {
            let StagedMutation {
                transaction_id: mutation_transaction_id,
                table_id,
                table_type,
                user_id,
                operation_kind,
                primary_key,
                payload,
                tombstone,
                ..
            } = mutation;

            if &mutation_transaction_id != transaction_id {
                return Err(TransactionAccessError::invalid_operation(format!(
                    "staged mutation transaction mismatch: expected '{}', got '{}'",
                    transaction_id, mutation_transaction_id
                )));
            }

            self.stage_mutation(
                transaction_id,
                &table_id,
                table_type,
                user_id,
                operation_kind,
                primary_key,
                payload,
                tombstone,
            )?;
        }

        Ok(())
    }
}

/// Query-time transaction context shared between the coordinator and providers.
#[derive(Debug, Clone)]
pub struct TransactionQueryContext {
    pub transaction_id:     TransactionId,
    /// Live pin cell shared with [`crate::TransactionHandle`].
    snapshot_log_index:     Arc<AtomicU64>,
    pub overlay_view:       Arc<dyn TransactionOverlayView>,
    pub mutation_sink:      Arc<dyn TransactionMutationSink>,
    pub access_validator:   Arc<dyn TransactionAccessValidator>,
}

impl TransactionQueryContext {
    #[inline]
    pub fn new(
        transaction_id: TransactionId,
        snapshot_log_index: Arc<AtomicU64>,
        overlay_view: Arc<dyn TransactionOverlayView>,
        mutation_sink: Arc<dyn TransactionMutationSink>,
        access_validator: Arc<dyn TransactionAccessValidator>,
    ) -> Self {
        Self {
            transaction_id,
            snapshot_log_index,
            overlay_view,
            mutation_sink,
            access_validator,
        }
    }

    /// Pinned group frontier log index, or `0` when still unset (empty view).
    ///
    /// Tables still read this as `snapshot_commit_seq` for MVCC bounds.
    #[inline]
    pub fn snapshot_commit_seq(&self) -> u64 {
        let value = self.snapshot_log_index.load(Ordering::Acquire);
        if value == SNAPSHOT_UNSET {
            0
        } else {
            value
        }
    }

    #[inline]
    pub fn snapshot_log_index_cell(&self) -> &Arc<AtomicU64> {
        &self.snapshot_log_index
    }
}
