use std::{env, mem, sync::Arc, time::Instant};

use async_trait::async_trait;
use kalamdb_backend::{
    manager::BackendSessionManager,
    session::{BackendAuth, BackendSessionSnapshot},
};
use kalamdb_commons::models::{Role, SessionOrigin, TransactionId, TransactionOrigin, UserId};
use kalamdb_transactions::{
    ExecutionOwnerKey, TransactionEngine, TransactionEngineResult, TransactionHandle,
};
use tokio::runtime::Builder;

#[derive(Debug)]
struct NoopTransactionEngine;

#[async_trait]
impl TransactionEngine for NoopTransactionEngine {
    async fn begin(
        &self,
        _owner_key: ExecutionOwnerKey,
        _owner_id: Arc<str>,
        _origin: TransactionOrigin,
    ) -> TransactionEngineResult<TransactionId> {
        unreachable!("idle session memory helper does not begin transactions")
    }

    async fn commit(&self, _transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        unreachable!("idle session memory helper does not commit transactions")
    }

    async fn rollback(&self, _transaction_id: &TransactionId) -> TransactionEngineResult<()> {
        unreachable!("idle session memory helper does not roll back transactions")
    }

    fn active_for_owner(&self, _owner_key: &ExecutionOwnerKey) -> Option<TransactionId> {
        None
    }

    fn get_handle(&self, _transaction_id: &TransactionId) -> Option<TransactionHandle> {
        None
    }
}

fn auth(index: usize) -> BackendAuth {
    BackendAuth::new(UserId::new(format!("wire_user_{index}")), Role::User, "password", i64::MAX)
}

fn snapshot_heap_estimate(snapshot: &BackendSessionSnapshot) -> usize {
    mem::size_of::<BackendSessionSnapshot>()
        + snapshot.session_id.len()
        + snapshot.state.len()
        + snapshot.current_schema.as_ref().map_or(0, String::len)
        + snapshot.client_addr.as_ref().map_or(0, String::len)
        + snapshot.last_method.as_ref().map_or(0, String::len)
        + snapshot
            .authenticated_user_id
            .as_ref()
            .map_or(0, |user_id| user_id.as_str().len())
        + snapshot.transaction_id.as_ref().map_or(0, |tx_id| tx_id.as_str().len())
        + snapshot
            .transaction_state
            .map_or(0, |_| mem::size_of::<kalamdb_commons::models::TransactionState>())
}

#[cfg(unix)]
fn max_rss_bytes() -> u64 {
    let mut usage = mem::MaybeUninit::<libc::rusage>::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return 0;
    }
    let usage = unsafe { usage.assume_init() };
    #[cfg(target_os = "macos")]
    {
        usage.ru_maxrss as u64
    }
    #[cfg(not(target_os = "macos"))]
    {
        (usage.ru_maxrss as u64).saturating_mul(1024)
    }
}

#[cfg(not(unix))]
fn max_rss_bytes() -> u64 {
    0
}

fn main() {
    let session_count = env::var("KALAMDB_BENCH_SESSIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1_000);
    let manager = BackendSessionManager::new(Arc::new(NoopTransactionEngine));

    let open_started = Instant::now();
    for index in 0..session_count {
        manager
            .open_session(
                SessionOrigin::WireProtocol,
                format!("019dabfa-1538-7c23-8e61-{index:012x}"),
                auth(index),
                Some("default".to_string()),
                Some("127.0.0.1:5432".to_string()),
            )
            .expect("open idle backend session");
    }
    let open_elapsed = open_started.elapsed();

    let snapshot_started = Instant::now();
    let snapshots = manager.snapshot();
    let snapshot_elapsed = snapshot_started.elapsed();
    let estimated_snapshot_bytes = snapshots.iter().map(snapshot_heap_estimate).sum::<usize>();

    let runtime = Builder::new_current_thread().enable_time().build().expect("tokio runtime");
    let cleanup_started = Instant::now();
    let removed = runtime
        .block_on(manager.cleanup_expired_sessions(i64::MAX))
        .expect("cleanup expired idle sessions");
    let cleanup_elapsed = cleanup_started.elapsed();

    println!("sessions={session_count}");
    println!("open_elapsed_secs={:.6}", open_elapsed.as_secs_f64());
    println!("snapshot_elapsed_secs={:.6}", snapshot_elapsed.as_secs_f64());
    println!("snapshot_rows={}", snapshots.len());
    println!("snapshot_estimated_bytes={estimated_snapshot_bytes}");
    println!("cleanup_elapsed_secs={:.6}", cleanup_elapsed.as_secs_f64());
    println!("cleanup_removed_sessions={}", removed.len());
    println!("max_rss_bytes={}", max_rss_bytes());
}
