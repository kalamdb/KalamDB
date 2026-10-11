//! Data operation responses
//!
//! Shared response type for both user and shared data operations.
use kalamdb_commons::ids::{check_entry_slot_count, RaftVersionId, VersionError, VersionId};
use serde::{Deserialize, Serialize};

/// Assign one [`VersionId`] per final row slot in a committed Raft entry.
///
/// Ordinals are `0..slot_count` (not `mutation_order as u16`). Entries that need
/// more than 65 536 final versions are rejected.
pub fn assign_entry_versions(
    entry_log_index: u64,
    slot_count: usize,
) -> Result<Vec<VersionId>, VersionError> {
    let slots = slot_count as u64;
    check_entry_slot_count(slots)?;
    let mut versions = Vec::with_capacity(slot_count);
    for ordinal in 0..slot_count {
        let version = RaftVersionId::try_new(entry_log_index, ordinal as u32)?.version();
        versions.push(version);
    }
    Ok(versions)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TransactionApplyResult {
    pub rows_affected:      usize,
    /// Raft log index of the committed entry (group frontier, not a packed seq).
    pub log_index:          u64,
    pub notifications_sent: usize,
    pub manifest_updates:   usize,
    pub publisher_events:   usize,
}

/// Response for data operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum DataResponse {
    #[default]
    Ok,

    /// Number of rows affected by the operation
    RowsAffected(usize),

    /// Explicit transaction commit applied in one durable cycle.
    TransactionCommitted(TransactionApplyResult),

    /// Subscription registered
    Subscribed { subscription_id: String },

    /// Error
    Error { message: String },
}

impl DataResponse {
    /// Create an error response with the given message
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Error {
            message: msg.into(),
        }
    }

    /// Store a Raft error. Coded errors keep their code and arguments.
    pub fn from_raft_error(error: crate::error::RaftError) -> Self {
        match error {
            crate::error::RaftError::Coded(error) => Self::error(error.encode()),
            other => Self::error(other.to_string()),
        }
    }

    /// Returns true if this is not an error response
    pub fn is_ok(&self) -> bool {
        !matches!(self, Self::Error { .. })
    }

    /// Returns the number of rows affected, or 0 if not applicable
    pub fn rows_affected(&self) -> usize {
        match self {
            DataResponse::RowsAffected(n) => *n,
            DataResponse::TransactionCommitted(result) => result.rows_affected,
            _ => 0,
        }
    }

    pub fn committed_log_index(&self) -> Option<u64> {
        match self {
            DataResponse::TransactionCommitted(result) => Some(result.log_index),
            _ => None,
        }
    }

    /// Backward-compatible alias for [`Self::committed_log_index`].
    pub fn committed_commit_seq(&self) -> Option<u64> {
        self.committed_log_index()
    }

    pub fn committed_side_effect_counts(&self) -> Option<(usize, usize, usize)> {
        match self {
            DataResponse::TransactionCommitted(result) => {
                Some((result.notifications_sent, result.manifest_updates, result.publisher_events))
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_entry_versions_packs_log_index_and_ordinal() {
        let versions = assign_entry_versions(5, 2).unwrap();
        assert_eq!(versions[0], RaftVersionId::try_new(5, 0).unwrap().version());
        assert_eq!(versions[1], RaftVersionId::try_new(5, 1).unwrap().version());
        assert!(assign_entry_versions(1, 65_537).is_err());
    }

    #[test]
    fn test_data_response_is_ok() {
        assert!(DataResponse::Ok.is_ok());
        assert!(DataResponse::RowsAffected(5).is_ok());
        assert!(DataResponse::TransactionCommitted(TransactionApplyResult {
            rows_affected:      2,
            log_index:          9,
            notifications_sent: 1,
            manifest_updates:   1,
            publisher_events:   1,
        })
        .is_ok());
        assert!(DataResponse::Subscribed {
            subscription_id: "sub_1".to_string(),
        }
        .is_ok());
        assert!(!DataResponse::Error {
            message: "error".to_string(),
        }
        .is_ok());
    }

    #[test]
    fn test_data_response_rows_affected() {
        assert_eq!(DataResponse::Ok.rows_affected(), 0);
        assert_eq!(DataResponse::RowsAffected(10).rows_affected(), 10);
        assert_eq!(
            DataResponse::TransactionCommitted(TransactionApplyResult {
                rows_affected:      4,
                log_index:          11,
                notifications_sent: 2,
                manifest_updates:   2,
                publisher_events:   2,
            })
            .rows_affected(),
            4
        );
        assert_eq!(
            DataResponse::Error {
                message: "err".to_string(),
            }
            .rows_affected(),
            0
        );
    }

    #[test]
    fn test_data_response_committed_log_index() {
        assert_eq!(
            DataResponse::TransactionCommitted(TransactionApplyResult {
                rows_affected:      3,
                log_index:          42,
                notifications_sent: 1,
                manifest_updates:   1,
                publisher_events:   1,
            })
            .committed_log_index(),
            Some(42)
        );
        assert_eq!(DataResponse::RowsAffected(2).committed_log_index(), None);
    }

    #[test]
    fn test_data_response_committed_side_effect_counts() {
        assert_eq!(
            DataResponse::TransactionCommitted(TransactionApplyResult {
                rows_affected:      3,
                log_index:          42,
                notifications_sent: 5,
                manifest_updates:   3,
                publisher_events:   2,
            })
            .committed_side_effect_counts(),
            Some((5, 3, 2))
        );
        assert_eq!(DataResponse::RowsAffected(2).committed_side_effect_counts(), None);
    }

    #[test]
    fn test_data_response_error_constructor() {
        let response = DataResponse::error("test error");
        assert!(!response.is_ok());
        match response {
            DataResponse::Error { message } => assert_eq!(message, "test error"),
            _ => panic!("Expected error response"),
        }
    }
}
