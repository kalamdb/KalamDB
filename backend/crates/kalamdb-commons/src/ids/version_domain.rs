#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::TableId;

/// Exact identity of one ordered history. Numeric versions have no global order.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct VersionDomain {
    pub history_incarnation: String,
    pub table_id:            TableId,
    pub scope_id:            u64,
    /// Reserved for future partitioned tables; currently always absent.
    pub partition_id:        Option<u64>,
}

impl VersionDomain {
    pub fn new(history: impl Into<String>, table_id: TableId, scope_id: u64) -> Self {
        Self {
            history_incarnation: history.into(),
            table_id,
            scope_id,
            partition_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{same_domain_cmp, RaftVersionId, VersionError};

    #[test]
    fn rejects_every_cross_scope_comparison() {
        let domain = VersionDomain::new("history-a", TableId::new("app".into(), "a".into()), 2);
        let version = RaftVersionId::try_new(10, 0).unwrap().version();
        let mut alternatives = vec![domain.clone(); 4];
        alternatives[0].history_incarnation = "history-b".into();
        alternatives[1].table_id = TableId::new("app".into(), "b".into());
        alternatives[2].scope_id = 3;
        alternatives[3].partition_id = Some(1);
        for other in alternatives {
            assert_eq!(
                same_domain_cmp(&domain, version, &other, version),
                Err(VersionError::DomainMismatch)
            );
        }
        assert!(same_domain_cmp(&domain, version, &domain, version).is_ok());
    }
}
