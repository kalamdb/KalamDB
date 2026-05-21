use std::{future::Future, sync::Arc};

use kalamdb_commons::models::{TransactionId, TransactionOrigin};
use kalamdb_transactions::{ExecutionOwnerKey, RequestTransactionCoordinator};

use crate::{app_context::AppContext, error::KalamDbError};

pub use kalamdb_transactions::{
    RequestTransactionBatchGuard, RequestTransactionError, RequestTransactionState,
    OPEN_REQUEST_TRANSACTION_ROLLED_BACK_MESSAGE,
};

#[derive(Debug, Clone, Copy)]
pub struct AppContextRequestTransactionCoordinator<'a> {
    app_context: &'a AppContext,
}

impl<'a> AppContextRequestTransactionCoordinator<'a> {
    #[inline]
    pub fn new(app_context: &'a AppContext) -> Self {
        Self { app_context }
    }
}

impl RequestTransactionCoordinator for AppContextRequestTransactionCoordinator<'_> {
    type Error = KalamDbError;

    fn active_for_owner(&self, owner_key: &ExecutionOwnerKey) -> Option<TransactionId> {
        self.app_context.transaction_coordinator().active_for_owner(owner_key)
    }

    fn begin_sql_request(
        &self,
        owner_key: ExecutionOwnerKey,
        owner_id: Arc<str>,
    ) -> Result<TransactionId, Self::Error> {
        self.app_context.transaction_coordinator().begin(
            owner_key,
            owner_id,
            TransactionOrigin::SqlBatch,
        )
    }

    fn commit_request_transaction<'a>(
        &'a self,
        transaction_id: &'a TransactionId,
    ) -> impl Future<Output = Result<TransactionId, Self::Error>> + Send + 'a {
        async move {
            let committed = self.app_context.transaction_coordinator().commit(transaction_id).await?;
            Ok(committed.transaction_id)
        }
    }

    fn rollback_request_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<(), Self::Error> {
        self.app_context.transaction_coordinator().rollback(transaction_id)
    }
}

pub fn map_request_transaction_error(
    error: RequestTransactionError<KalamDbError>,
) -> KalamDbError {
    match error {
        RequestTransactionError::AlreadyActive {
            owner_id,
            transaction_id,
        } => KalamDbError::Conflict(format!(
            "request owner '{}' already has an active transaction '{}'",
            owner_id, transaction_id
        )),
        RequestTransactionError::NoActiveTransaction { operation } => KalamDbError::InvalidOperation(
            format!("{} requires an active explicit SQL transaction", operation),
        ),
        RequestTransactionError::RequestCompletedOpen => KalamDbError::InvalidOperation(
            OPEN_REQUEST_TRANSACTION_ROLLED_BACK_MESSAGE.to_string(),
        ),
        RequestTransactionError::Coordinator(error) => error,
    }
}
