use std::{
    collections::HashMap,
    env,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use kalamdb_backend::{manager::BackendSessionManager, session::BackendAuth};
use kalamdb_commons::models::{Role, SessionOrigin, TransactionId, TransactionOrigin, UserId};
use kalamdb_transactions::{
    ExecutionOwnerKey, TransactionEngine, TransactionEngineError, TransactionEngineResult,
    TransactionHandle, TransactionRaftBinding,
};
use tokio::runtime::Builder;
use uuid::Uuid;

#[derive(Debug, Default)]
struct FakeTransactionEngine {
    active_by_owner: Mutex<HashMap<ExecutionOwnerKey, TransactionId>>,
    handles: Mutex<HashMap<TransactionId, TransactionHandle>>,
}

#[async_trait]
impl TransactionEngine for FakeTransactionEngine {
    async fn begin(
        &self,
        owner_key: ExecutionOwnerKey,
        owner_id: Arc<str>,
        origin: TransactionOrigin,
    ) -> TransactionEngineResult<TransactionId> {
        let mut active_by_owner = self
            .active_by_owner
            .lock()
            .map_err(|_| TransactionEngineError::operation("active map poisoned"))?;
        if active_by_owner.contains_key(&owner_key) {
            return Err(TransactionEngineError::operation("owner already active"));
        }

        let transaction_id = TransactionId::new(Uuid::now_v7().to_string());
        let handle = TransactionHandle::new(
            transaction_id.clone(),
            owner_key.clone(),
            owner_id,
            origin,
            TransactionRaftBinding::LocalSingleNode,
            0,
            Instant::now(),
        );
        self.handles
            .lock()
            .map_err(|_| TransactionEngineError::operation("handle map poisoned"))?
            .insert(transaction_id.clone(), handle);
        active_by_owner.insert(owner_key, transaction_id.clone());
        Ok(transaction_id)
    }

    async fn commit(&self, transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        self.finish(transaction_id)
    }

    async fn rollback(&self, transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        self.finish(transaction_id)
    }

    fn active_for_owner(&self, owner_key: &ExecutionOwnerKey) -> Option<TransactionId> {
        self.active_by_owner
            .lock()
            .ok()
            .and_then(|active_by_owner| active_by_owner.get(owner_key).cloned())
    }

    fn get_handle(&self, transaction_id: &TransactionId) -> Option<TransactionHandle> {
        self.handles
            .lock()
            .ok()
            .and_then(|handles| handles.get(transaction_id).cloned())
    }
}

impl FakeTransactionEngine {
    fn finish(&self, transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        let handle = self
            .handles
            .lock()
            .map_err(|_| TransactionEngineError::operation("handle map poisoned"))?
            .remove(transaction_id);

        if let Some(handle) = handle {
            self.active_by_owner
                .lock()
                .map_err(|_| TransactionEngineError::operation("active map poisoned"))?
                .remove(&handle.owner_key);
        }
        Ok(())
    }
}

fn auth() -> BackendAuth {
    BackendAuth::new(UserId::new("wire_user"), Role::User, "password", i64::MAX)
}

async fn run_commit_cycles(manager: &BackendSessionManager, iterations: usize) {
    for _ in 0..iterations {
        manager.begin_block("019dabfa-1538-7c23-8e61-de751d8c1c38").await.unwrap();
        manager.commit_block("019dabfa-1538-7c23-8e61-de751d8c1c38").await.unwrap();
    }
}

async fn run_rollback_cycles(manager: &BackendSessionManager, iterations: usize) {
    for _ in 0..iterations {
        manager.begin_block("019dabfa-1538-7c23-8e61-de751d8c1c38").await.unwrap();
        manager.rollback_block("019dabfa-1538-7c23-8e61-de751d8c1c38").await.unwrap();
    }
}

fn main() {
    let iterations = env::var("KALAMDB_BENCH_BLOCK_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1_000);

    let runtime = Builder::new_current_thread().enable_time().build().expect("tokio runtime");
    let manager = BackendSessionManager::new(Arc::new(FakeTransactionEngine::default()));
    manager
        .open_session(
            SessionOrigin::WireProtocol,
            "019dabfa-1538-7c23-8e61-de751d8c1c38",
            auth(),
            Some("default".to_string()),
            Some("127.0.0.1:5432".to_string()),
        )
        .expect("open backend session");

    let commit_started = Instant::now();
    runtime.block_on(run_commit_cycles(&manager, iterations));
    let commit_elapsed = commit_started.elapsed();

    let rollback_started = Instant::now();
    runtime.block_on(run_rollback_cycles(&manager, iterations));
    let rollback_elapsed = rollback_started.elapsed();

    println!("iterations={iterations}");
    println!("commit_cycles_elapsed_secs={:.6}", commit_elapsed.as_secs_f64());
    println!("commit_cycle_avg_secs={:.9}", commit_elapsed.as_secs_f64() / iterations as f64);
    println!("rollback_cycles_elapsed_secs={:.6}", rollback_elapsed.as_secs_f64());
    println!(
        "rollback_cycle_avg_secs={:.9}",
        rollback_elapsed.as_secs_f64() / iterations as f64
    );
    println!("ended_at_ms={}", now_ms());
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}
