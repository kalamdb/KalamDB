use kalamdb_backend::manager::{BackendSessionError, BackendSessionManager};
use kalamdb_commons::models::TransactionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireTransactionControl {
    Begin { read_only: bool },
    Commit,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireTransactionOutcome {
    Began { transaction_id: TransactionId },
    Committed,
    RolledBack,
}

/// Classify transaction control statements without a full SQL parse.
///
/// Wire queries that are not tx control return `None` and fall through to the
/// normal classify + execute path (single parse via `prepare_statement_metadata`).
pub fn classify_transaction_control(sql: &str) -> Option<WireTransactionControl> {
    let upper = sql.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return None;
    }

    if upper == "COMMIT" || upper.starts_with("COMMIT ") {
        return Some(WireTransactionControl::Commit);
    }
    if upper == "ROLLBACK" || upper.starts_with("ROLLBACK ") {
        return Some(WireTransactionControl::Rollback);
    }
    if upper == "BEGIN" || upper.starts_with("BEGIN ") || upper.starts_with("START TRANSACTION") {
        return Some(WireTransactionControl::Begin {
            read_only: upper.contains("READ ONLY"),
        });
    }

    None
}

pub async fn execute_transaction_control(
    manager: &BackendSessionManager,
    session_id: &str,
    control: WireTransactionControl,
) -> Result<WireTransactionOutcome, BackendSessionError> {
    match control {
        WireTransactionControl::Begin { .. } => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_begin_variants() {
        assert_eq!(
            classify_transaction_control("BEGIN"),
            Some(WireTransactionControl::Begin { read_only: false })
        );
        assert_eq!(
            classify_transaction_control("START TRANSACTION READ ONLY"),
            Some(WireTransactionControl::Begin { read_only: true })
        );
    }

    #[test]
    fn classifies_commit_and_rollback() {
        assert_eq!(classify_transaction_control("COMMIT"), Some(WireTransactionControl::Commit));
        assert_eq!(
            classify_transaction_control("ROLLBACK"),
            Some(WireTransactionControl::Rollback)
        );
    }

    #[test]
    fn ignores_non_control_sql() {
        assert_eq!(classify_transaction_control("SELECT 1"), None);
        assert_eq!(classify_transaction_control("BEGIN; SELECT 1"), None);
    }
}
