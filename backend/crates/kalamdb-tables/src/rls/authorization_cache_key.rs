use kalamdb_commons::{PolicyId, TableId, UserId};

/// Identity- and generation-complete key for an authorization set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AuthorizationCacheKey {
    pub policy_id: PolicyId,
    pub policy_generation: u64,
    pub principal_id: UserId,
    pub relation_table: TableId,
    pub relation_generation: u64,
    pub snapshot_commit_seq: Option<u64>,
}
