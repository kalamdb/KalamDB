mod support;

use kalamdb_backend::manager::{BackendSessionError, BackendSessionManager};
use kalamdb_commons::models::SessionOrigin;
use support::{auth, auth_with_lease, FakeTransactionEngine};

#[tokio::test]
#[ntest::timeout(5_000)]
async fn double_begin_is_rejected_while_live_block_exists() {
    let engine = FakeTransactionEngine::new();
    let manager = BackendSessionManager::new(engine);
    let session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    manager
        .open_session(SessionOrigin::WireProtocol, session_id, auth("user_1"), None, None)
        .unwrap();
    manager.begin_block(session_id).await.unwrap();

    let error = manager.begin_block(session_id).await.unwrap_err();
    assert_eq!(error, BackendSessionError::ActiveBlock(session_id.to_string()));
}

#[tokio::test]
#[ntest::timeout(5_000)]
async fn failed_block_rejects_work_until_rollback() {
    let engine = FakeTransactionEngine::new();
    let manager = BackendSessionManager::new(engine);
    let session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    manager
        .open_session(SessionOrigin::WireProtocol, session_id, auth("user_1"), None, None)
        .unwrap();
    manager.begin_block(session_id).await.unwrap();
    manager.mark_statement_failed(session_id).unwrap();

    let error = manager.ensure_block_allows_work(session_id).unwrap_err();
    assert_eq!(error, BackendSessionError::FailedBlock(session_id.to_string()));

    manager.rollback_block(session_id).await.unwrap();
    manager.ensure_block_allows_work(session_id).unwrap();
}

#[tokio::test]
#[ntest::timeout(5_000)]
async fn close_session_rolls_back_open_block_before_removal() {
    let engine = FakeTransactionEngine::new();
    let manager = BackendSessionManager::new(engine.clone());
    let session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    manager
        .open_session(SessionOrigin::WireProtocol, session_id, auth("user_1"), None, None)
        .unwrap();
    let transaction_id = manager.begin_block(session_id).await.unwrap();

    manager.close_session(session_id).await.unwrap();

    assert!(engine.state(&transaction_id).is_none());
    assert!(manager.snapshot().is_empty());
    assert_eq!(manager.cleanup_counters().closed_sessions, 1);
}

#[tokio::test]
#[ntest::timeout(5_000)]
async fn stale_pinned_transaction_is_cleared_before_new_begin() {
    let engine = FakeTransactionEngine::new();
    let manager = BackendSessionManager::new(engine.clone());
    let session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    manager
        .open_session(SessionOrigin::WireProtocol, session_id, auth("user_1"), None, None)
        .unwrap();
    let stale = manager.begin_block(session_id).await.unwrap();
    engine.drop_transaction(&stale);

    let next = manager.begin_block(session_id).await.unwrap();

    assert_ne!(stale, next);
    assert_eq!(manager.snapshot()[0].transaction_id.as_ref(), Some(&next));
}

#[tokio::test]
#[ntest::timeout(5_000)]
async fn cleanup_expired_sessions_rolls_back_open_block() {
    let engine = FakeTransactionEngine::new();
    let manager = BackendSessionManager::new(engine.clone());
    let session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    manager
        .open_session(
            SessionOrigin::WireProtocol,
            session_id,
            auth_with_lease("user_1", 10),
            None,
            None,
        )
        .unwrap();
    let transaction_id = manager.begin_block(session_id).await.unwrap();

    let removed = manager.cleanup_expired_sessions(11).await.unwrap();

    assert_eq!(removed, vec![session_id.to_string()]);
    assert_eq!(manager.cleanup_counters().runs, 1);
    assert_eq!(manager.cleanup_counters().removed_sessions, 1);
    assert_eq!(manager.cleanup_counters().closed_sessions, 1);
    assert!(engine.state(&transaction_id).is_none());
    assert!(manager.snapshot().is_empty());
}
