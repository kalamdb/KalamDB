use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_commons::models::{TransactionId, TransactionOrigin};
use thiserror::Error;

use crate::{ExecutionOwnerKey, TransactionHandle};

#[derive(Debug, Error)]
pub enum TransactionEngineError {
    #[error("{0}")]
    Operation(String),
}

impl TransactionEngineError {
    pub fn operation(error: impl ToString) -> Self {
        Self::Operation(error.to_string())
    }
}

pub type TransactionEngineResult<T> = Result<T, TransactionEngineError>;

#[async_trait]
pub trait TransactionEngine: Send + Sync {
    async fn begin(
        &self,
        owner_key: ExecutionOwnerKey,
        owner_id: Arc<str>,
        origin: TransactionOrigin,
    ) -> TransactionEngineResult<TransactionId>;

    async fn commit(&self, transaction_id: &TransactionId) -> TransactionEngineResult<()>;

    async fn rollback(&self, transaction_id: &TransactionId) -> TransactionEngineResult<()>;

    fn active_for_owner(&self, owner_key: &ExecutionOwnerKey) -> Option<TransactionId>;

    fn get_handle(&self, transaction_id: &TransactionId) -> Option<TransactionHandle>;
}
