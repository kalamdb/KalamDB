use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_commons::{
    ids::check_entry_slot_count,
    models::{TransactionId, TransactionOrigin},
    TableType,
};
use kalamdb_sharding::GroupId;
use thiserror::Error;

use crate::{ExecutionOwnerKey, StagedMutation, TransactionHandle};

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

/// Why a versioned commit was refused before any storage write.
#[derive(Debug, Error)]
pub enum CommitAdmissionError {
    #[error("stream tables are not supported inside explicit transactions")]
    Stream,
    #[error("system tables are not supported inside explicit transactions")]
    System,
    #[error("user transaction batch mutation missing user_id")]
    MissingUser,
    #[error("transactional access to table '{table}' did not resolve a commit group")]
    UnresolvedGroup { table: String },
    #[error(
        "explicit transactions must remain within one data raft group; '{table}' mapped to {group:?} while prior mutations mapped to {prior:?}"
    )]
    CrossDomain {
        table: String,
        group: GroupId,
        prior: GroupId,
    },
    #[error("table '{table}' of type {table_type:?} cannot commit in raft group {group:?}")]
    IncompatibleBackend {
        table: String,
        table_type: TableType,
        group: GroupId,
    },
    #[error(
        "transaction buffer of {bytes} bytes exceeds max_transaction_buffer_bytes {max_bytes}"
    )]
    BufferLimit { bytes: usize, max_bytes: usize },
    #[error("{0}")]
    VersionLimit(String),
    #[error("commit admission group count {groups} does not match {mutations} mutations")]
    GroupCount { groups: usize, mutations: usize },
}

/// Reject a versioned commit that crosses domains, uses an incompatible backend,
/// or exceeds the byte or version-slot limit.
///
/// `groups[i]` is the data raft group for `mutations[i]`. Stream and system
/// mutations are rejected from `table_type` before that group is used. Distinct
/// `GroupId`s are different commit domains. Callers must do this before any
/// `StorageBackend::batch`.
pub fn admit_versioned_commit(
    mutations: &[StagedMutation],
    groups: &[Option<GroupId>],
    max_bytes: usize,
    version_slots: u64,
) -> Result<(), CommitAdmissionError> {
    if mutations.is_empty() {
        return Ok(());
    }
    if groups.len() != mutations.len() {
        return Err(CommitAdmissionError::GroupCount {
            groups: groups.len(),
            mutations: mutations.len(),
        });
    }

    let mut bound: Option<GroupId> = None;
    for (mutation, group) in mutations.iter().zip(groups.iter().copied()) {
        let resolved = match mutation.table_type {
            TableType::Stream => return Err(CommitAdmissionError::Stream),
            TableType::System => return Err(CommitAdmissionError::System),
            TableType::User => {
                if mutation.user_id.is_none() {
                    return Err(CommitAdmissionError::MissingUser);
                }
                let Some(group) = group else {
                    return Err(CommitAdmissionError::UnresolvedGroup {
                        table: mutation.table_id.to_string(),
                    });
                };
                if !matches!(group, GroupId::DataUserShard(_)) {
                    return Err(CommitAdmissionError::IncompatibleBackend {
                        table: mutation.table_id.to_string(),
                        table_type: mutation.table_type,
                        group,
                    });
                }
                group
            },
            TableType::Shared => {
                let Some(group) = group else {
                    return Err(CommitAdmissionError::UnresolvedGroup {
                        table: mutation.table_id.to_string(),
                    });
                };
                if !matches!(group, GroupId::DataSharedShard(_)) {
                    return Err(CommitAdmissionError::IncompatibleBackend {
                        table: mutation.table_id.to_string(),
                        table_type: mutation.table_type,
                        group,
                    });
                }
                group
            },
        };
        match bound {
            None => bound = Some(resolved),
            Some(prior) if prior != resolved => {
                return Err(CommitAdmissionError::CrossDomain {
                    table: mutation.table_id.to_string(),
                    group: resolved,
                    prior,
                });
            },
            Some(_) => {},
        }
    }

    let slots = version_slots.max(mutations.len() as u64);
    check_entry_slot_count(slots)
        .map_err(|error| CommitAdmissionError::VersionLimit(error.to_string()))?;
    let bytes = mutations.iter().map(StagedMutation::approximate_size_bytes).sum::<usize>();
    if bytes > max_bytes {
        return Err(CommitAdmissionError::BufferLimit { bytes, max_bytes });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kalamdb_commons::{
        models::{rows::Row, OperationKind, SchemaId, TableId, TransactionId, UserId},
        TableType,
    };
    use kalamdb_sharding::GroupId;

    use super::*;

    fn mutation(table_type: TableType, user_id: Option<&str>) -> StagedMutation {
        StagedMutation::new(
            TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d"),
            TableId::new(SchemaId::default(), "items".into()),
            table_type,
            user_id.map(UserId::new),
            OperationKind::Insert,
            "1",
            Row::new(BTreeMap::new()),
            false,
        )
    }

    #[test]
    fn same_user_shard_is_admitted() {
        let mutation = mutation(TableType::User, Some("user-a"));
        admit_versioned_commit(
            std::slice::from_ref(&mutation),
            &[Some(GroupId::DataUserShard(3))],
            usize::MAX,
            1,
        )
        .expect("one user shard is one commit domain");
    }

    #[test]
    fn cross_domain_groups_are_rejected() {
        let mutations = vec![
            mutation(TableType::User, Some("user-a")),
            mutation(TableType::User, Some("user-b")),
        ];
        let error = admit_versioned_commit(
            &mutations,
            &[
                Some(GroupId::DataUserShard(0)),
                Some(GroupId::DataUserShard(1)),
            ],
            usize::MAX,
            2,
        )
        .expect_err("two user shards are two domains");
        assert!(error.to_string().contains("one data raft group"), "{error}");
    }

    #[test]
    fn stream_and_system_are_incompatible() {
        let stream = mutation(TableType::Stream, Some("user-a"));
        let stream_error = admit_versioned_commit(
            std::slice::from_ref(&stream),
            &[Some(GroupId::DataUserShard(0))],
            usize::MAX,
            1,
        )
        .expect_err("stream");
        assert_eq!(
            stream_error.to_string(),
            "stream tables are not supported inside explicit transactions"
        );

        let system = mutation(TableType::System, None);
        let system_error =
            admit_versioned_commit(std::slice::from_ref(&system), &[None], usize::MAX, 1)
                .expect_err("system");
        assert_eq!(
            system_error.to_string(),
            "system tables are not supported inside explicit transactions"
        );
    }

    #[test]
    fn user_mutation_in_a_shared_group_is_incompatible() {
        let mutation = mutation(TableType::User, Some("user-a"));
        let error = admit_versioned_commit(
            std::slice::from_ref(&mutation),
            &[Some(GroupId::DataSharedShard(0))],
            usize::MAX,
            1,
        )
        .expect_err("wrong group kind");
        assert!(error.to_string().contains("cannot commit in raft group"), "{error}");
    }

    #[test]
    fn version_and_byte_limits_fail_before_a_write() {
        let mutation = mutation(TableType::User, Some("user-a"));
        let slots = admit_versioned_commit(
            std::slice::from_ref(&mutation),
            &[Some(GroupId::DataUserShard(0))],
            usize::MAX,
            65_537,
        )
        .expect_err("too many slots");
        assert!(slots.to_string().contains("65536"), "{slots}");

        let bytes = admit_versioned_commit(
            std::slice::from_ref(&mutation),
            &[Some(GroupId::DataUserShard(0))],
            0,
            1,
        )
        .expect_err("byte limit");
        assert!(bytes.to_string().contains("max_transaction_buffer_bytes"), "{bytes}");
    }
}
