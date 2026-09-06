use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::models::{Role, TransactionId, TransactionOrigin, TransactionState, UserId};
use kalamdb_transactions::{
    ExecutionOwnerKey, TransactionEngine, TransactionEngineError, TransactionEngineResult,
    TransactionHandle, TransactionRaftBinding,
};
use uuid::Uuid;

#[derive(Debug)]
pub struct FakeTransactionEngine {
    active_by_owner: DashMap<ExecutionOwnerKey, TransactionId>,
    handles:         DashMap<TransactionId, TransactionHandle>,
}

impl FakeTransactionEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            active_by_owner: DashMap::new(),
            handles:         DashMap::new(),
        })
    }

    pub fn state(&self, transaction_id: &TransactionId) -> Option<TransactionState> {
        self.handles.get(transaction_id).map(|handle| handle.state)
    }

    pub fn drop_transaction(&self, transaction_id: &TransactionId) {
        if let Some((_, handle)) = self.handles.remove(transaction_id) {
            self.active_by_owner.remove(&handle.owner_key);
        }
    }
}

#[async_trait]
impl TransactionEngine for FakeTransactionEngine {
    async fn begin(
        &self,
        owner_key: ExecutionOwnerKey,
        owner_id: Arc<str>,
        origin: TransactionOrigin,
    ) -> TransactionEngineResult<TransactionId> {
        if self.active_by_owner.contains_key(&owner_key) {
            return Err(TransactionEngineError::operation("owner already active"));
        }

        let transaction_id = TransactionId::new(Uuid::now_v7().to_string());
        let handle = TransactionHandle::new(
            transaction_id.clone(),
            owner_key,
            owner_id,
            origin,
            TransactionRaftBinding::LocalSingleNode,
            0,
            Instant::now(),
        );
        self.handles.insert(transaction_id.clone(), handle);
        self.active_by_owner.insert(owner_key, transaction_id.clone());
        Ok(transaction_id)
    }

    async fn commit(&self, transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        self.drop_transaction(transaction_id);
        Ok(())
    }

    async fn rollback(&self, transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        self.drop_transaction(transaction_id);
        Ok(())
    }

    fn active_for_owner(&self, owner_key: &ExecutionOwnerKey) -> Option<TransactionId> {
        self.active_by_owner.get(owner_key).map(|transaction_id| transaction_id.clone())
    }

    fn get_handle(&self, transaction_id: &TransactionId) -> Option<TransactionHandle> {
        self.handles.get(transaction_id).map(|handle| handle.clone())
    }
}

pub fn auth(user_id: &str) -> BackendAuth {
    auth_with_lease(user_id, i64::MAX)
}

pub fn auth_with_lease(user_id: &str, lease_expires_at_ms: i64) -> BackendAuth {
    BackendAuth::new(UserId::new(user_id), Role::User, "password", lease_expires_at_ms)
}
