use kalamdb_backend::manager::{BackendSessionError, BackendSessionManager};
use kalamdb_commons::models::TransactionId;

/// Connection transaction commands.
///
/// The shared classifier decides which SQL is `BEGIN`, `COMMIT`, or `ROLLBACK`.
/// This module only applies that decision to the connection's session block.
/// HTTP `/api/sql` uses the request transaction guard for the same statement kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireTransactionControl {
    Begin,
    Commit,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireTransactionOutcome {
    Began { transaction_id: TransactionId },
    Committed,
    RolledBack,
}

pub async fn execute_transaction_control(
    manager: &BackendSessionManager,
    session_id: &str,
    control: WireTransactionControl,
) -> Result<WireTransactionOutcome, BackendSessionError> {
    match control {
        WireTransactionControl::Begin => {
            let transaction_id = manager.begin_block(session_id).await?;
            Ok(WireTransactionOutcome::Began { transaction_id })
        },
        WireTransactionControl::Commit => {
            manager.commit_block(session_id).await?;
            Ok(WireTransactionOutcome::Committed)
        },
        WireTransactionControl::Rollback => {
            manager.rollback_block(session_id).await?;
            Ok(WireTransactionOutcome::RolledBack)
        },
    }
}
