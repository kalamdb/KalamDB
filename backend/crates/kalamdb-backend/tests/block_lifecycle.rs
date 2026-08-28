mod support;

use kalamdb_backend::manager::BackendSessionManager;
use kalamdb_commons::models::{SessionOrigin, TransactionState};
use support::{auth, FakeTransactionEngine};

#[tokio::test]
#[ntest::timeout(5_000)]
async fn begin_commit_then_begin_again_reuses_session_without_leaking_state() {
    let engine = FakeTransactionEngine::new();
    let manager = BackendSessionManager::new(engine.clone());
    let session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    manager
        .open_session(SessionOrigin::WireProtocol, session_id, auth("user_1"), None, None)
        .unwrap();

    let first = manager.begin_block(session_id).await.unwrap();
    assert_eq!(engine.state(&first), Some(TransactionState::OpenRead));
    manager.commit_block(session_id).await.unwrap();
    assert!(engine.state(&first).is_none());
    assert!(manager.snapshot()[0].transaction_id.is_none());

    let second = manager.begin_block(session_id).await.unwrap();
    assert_ne!(first, second);
    assert_eq!(engine.state(&second), Some(TransactionState::OpenRead));
    assert_eq!(manager.snapshot()[0].transaction_id.as_ref(), Some(&second));
}

#[tokio::test]
#[ntest::timeout(5_000)]
async fn concurrent_sessions_keep_independent_transaction_ids() {
    let engine = FakeTransactionEngine::new();
    let manager = BackendSessionManager::new(engine.clone());

    manager
        .open_session(
            SessionOrigin::WireProtocol,
            "019dabfa-1538-7c23-8e61-de751d8c1c38",
            auth("user_1"),
            None,
            None,
        )
        .unwrap();
    manager
        .open_session(
            SessionOrigin::WireProtocol,
            "019dabfa-1538-7c23-8e61-de751d8c1c39",
            auth("user_2"),
            None,
            None,
        )
        .unwrap();

    let first = manager.begin_block("019dabfa-1538-7c23-8e61-de751d8c1c38").await.unwrap();
    let second = manager.begin_block("019dabfa-1538-7c23-8e61-de751d8c1c39").await.unwrap();

    assert_ne!(first, second);
    let snapshots = manager.snapshot();
    assert_eq!(snapshots.len(), 2);
    assert!(snapshots
        .iter()
        .any(|snapshot| snapshot.transaction_id.as_ref() == Some(&first)));
    assert!(snapshots
        .iter()
        .any(|snapshot| snapshot.transaction_id.as_ref() == Some(&second)));
}
