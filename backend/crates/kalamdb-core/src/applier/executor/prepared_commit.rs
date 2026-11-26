//! One prepared commit for a versioned user or shared transaction.
//!
//! Storage operations are built before any table write. Post-commit work runs
//! only after `StorageBackend::batch` returns `Ok`.

use std::sync::Arc;

use kalamdb_commons::{
    ids::{SharedTableRowId, UserTableRowId},
    models::UserId,
    schemas::TableType,
    TableId,
};
use kalamdb_store::Operation;
use kalamdb_system::FileRef;
use kalamdb_tables::{KalamTableProvider, PlannedScopeFinish, SharedTableRow, UserTableRow};

use crate::transactions::CommitSideEffectPlan;

/// Files replaced or deleted by a committed row, removed after the batch succeeds.
pub(crate) struct FileCleanupIntent {
    pub table_id: TableId,
    pub table_type: TableType,
    pub user_id: Option<UserId>,
    pub file_refs: Vec<FileRef>,
}

/// Vector staging and manifest pending-write work for one planned mutation group.
pub(crate) enum CommitFinish {
    UserInsert {
        provider: Arc<KalamTableProvider>,
        user_id: UserId,
        entries: Vec<(UserTableRowId, UserTableRow)>,
    },
    SharedInsert {
        provider: Arc<KalamTableProvider>,
        entries: Vec<(SharedTableRowId, SharedTableRow)>,
    },
}

/// One table scope's post-commit work. `ops` is empty when the table has no vector columns.
pub(crate) struct ScopeWork<T> {
    key: u64,
    user: Option<UserId>,
    ops: Vec<T>,
}

/// Group `op` into the existing scope, or append a scope. Returns the scope index.
pub(crate) fn absorb_scope_work<T>(
    groups: &mut Vec<ScopeWork<T>>,
    key: u64,
    user: Option<UserId>,
    op: Option<T>,
) -> usize {
    if let Some(index) = groups.iter().position(|group| group.key == key && group.user == user) {
        if let Some(op) = op {
            groups[index].ops.push(op);
        }
        return index;
    }
    let index = groups.len();
    groups.push(ScopeWork {
        key,
        user,
        ops: op.into_iter().collect(),
    });
    index
}

impl CommitFinish {
    pub(crate) async fn run(self) {
        match self {
            Self::UserInsert {
                provider,
                user_id,
                entries,
            } => {
                let _ = provider.finish_user_insert_batch(&user_id, &entries).await;
            },
            Self::SharedInsert { provider, entries } => {
                let _ = provider.finish_shared_insert_batch(&entries).await;
            },
        }
    }
}

/// Effects that must not run until the commit batch is accepted.
pub(crate) struct PostCommitEffectPlan {
    pub notifications: CommitSideEffectPlan,
    pub file_cleanup_intents: Vec<FileCleanupIntent>,
}

/// Validated table writes for one commit domain, plus the effects that follow them.
pub(crate) struct PreparedCommitBatch {
    pub log_index: u64,
    pub storage_operations: Vec<Operation>,
    pub rows_affected: usize,
    pub post_commit_effects: PostCommitEffectPlan,
    pub finishes: Vec<CommitFinish>,
    pub user_scopes: Vec<ScopeWork<PlannedScopeFinish>>,
    pub user_scope_providers: Vec<Arc<KalamTableProvider>>,
    pub shared_scopes: Vec<ScopeWork<PlannedScopeFinish>>,
    pub shared_scope_providers: Vec<Arc<KalamTableProvider>>,
    pub frontier_table_id: TableId,
    pub frontier_table_type: TableType,
    pub frontier_user_id: Option<UserId>,
}

impl PreparedCommitBatch {
    pub(crate) fn push_user_scope(
        &mut self,
        provider: Arc<KalamTableProvider>,
        user_id: UserId,
        op: Option<PlannedScopeFinish>,
    ) {
        let key = Arc::as_ptr(&provider) as u64;
        let before = self.user_scopes.len();
        let index = absorb_scope_work(&mut self.user_scopes, key, Some(user_id), op);
        if index == before {
            self.user_scope_providers.push(provider);
        }
    }

    pub(crate) fn push_shared_scope(
        &mut self,
        provider: Arc<KalamTableProvider>,
        op: Option<PlannedScopeFinish>,
    ) {
        let key = Arc::as_ptr(&provider) as u64;
        let before = self.shared_scopes.len();
        let index = absorb_scope_work(&mut self.shared_scopes, key, None, op);
        if index == before {
            self.shared_scope_providers.push(provider);
        }
    }

    /// One pending-write mark per scope. Vector rows stay in commit order inside that scope.
    pub(crate) async fn run_scope_finishes(&mut self) {
        debug_assert_eq!(self.user_scopes.len(), self.user_scope_providers.len());
        debug_assert_eq!(self.shared_scopes.len(), self.shared_scope_providers.len());
        let user_scopes = std::mem::take(&mut self.user_scopes);
        let user_providers = std::mem::take(&mut self.user_scope_providers);
        for (work, provider) in user_scopes.into_iter().zip(user_providers) {
            let Some(user_id) = work.user else {
                continue;
            };
            let _ = provider.finish_user_scope(&user_id, &work.ops).await;
        }
        let shared_scopes = std::mem::take(&mut self.shared_scopes);
        let shared_providers = std::mem::take(&mut self.shared_scope_providers);
        for (work, provider) in shared_scopes.into_iter().zip(shared_providers) {
            let _ = provider.finish_shared_scope(&work.ops).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_scope_group_per_table_and_user() {
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");
        let mut groups = Vec::new();
        for _ in 0..4 {
            absorb_scope_work(&mut groups, 1, Some(alice.clone()), None::<u8>);
        }
        absorb_scope_work(&mut groups, 1, Some(alice.clone()), Some(9));
        absorb_scope_work(&mut groups, 1, Some(bob), Some(4));
        absorb_scope_work(&mut groups, 2, None, Some(1));
        absorb_scope_work(&mut groups, 2, None, None::<u8>);

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].ops, vec![9]);
        assert_eq!(groups[1].ops, vec![4]);
        assert_eq!(groups[2].ops, vec![1]);
        assert_eq!(groups[0].user.as_ref().map(UserId::as_str), Some("alice"));
        assert!(groups[2].user.is_none());
    }
}
