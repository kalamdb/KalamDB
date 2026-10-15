mod support;

use std::sync::Arc;

use kalamdb_commons::TransactionState;
use ntest::timeout;
use support::{
    create_cluster_app_context, create_executor, create_shared_table, execute_err, execute_ok,
    insert_sql, request_exec_ctx, request_transaction_coordinator, request_transaction_state,
    select_names, unique_namespace,
};

#[tokio::test]
#[timeout(30000)]
async fn savepoints_and_rollback_to_savepoint_recover_aborted_transaction_block() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let namespace = unique_namespace("pg18_tx_savepoint");
    let table_id = create_shared_table(&app_ctx, &namespace, "ledger").await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let req_ctx = request_exec_ctx(&app_ctx, "pg18-savepoint-session");
    let observer_ctx = request_exec_ctx(&app_ctx, "pg18-savepoint-observer");

    execute_ok(&executor, &req_ctx, "BEGIN ISOLATION LEVEL REPEATABLE READ").await;
    execute_ok(&executor, &req_ctx, &insert_sql(&table_id, 1, "before_sp")).await;

    execute_ok(&executor, &req_ctx, "SAVEPOINT sp1").await;
    execute_ok(&executor, &req_ctx, &insert_sql(&table_id, 2, "inside_sp1")).await;

    execute_ok(&executor, &req_ctx, "SAVEPOINT sp2").await;
    execute_ok(&executor, &req_ctx, &insert_sql(&table_id, 3, "inside_sp2")).await;

    assert_eq!(
        select_names(&executor, &req_ctx, &table_id).await,
        vec![
            "before_sp".to_string(),
            "inside_sp1".to_string(),
            "inside_sp2".to_string()
        ]
    );

    // Roll back to sp2 (discards row 3, keeps sp2 active)
    execute_ok(&executor, &req_ctx, "ROLLBACK TO SAVEPOINT sp2").await;
    assert_eq!(
        select_names(&executor, &req_ctx, &table_id).await,
        vec!["before_sp".to_string(), "inside_sp1".to_string()]
    );

    // Release sp2
    execute_ok(&executor, &req_ctx, "RELEASE SAVEPOINT sp2").await;

    // Cause a runtime error inside the transaction (select from nonexistent column)
    let bad_err = executor
        .execute(
            &format!(
                "SELECT nonexistent_col FROM {}.{}",
                table_id.namespace_id(),
                table_id.table_name()
            ),
            &req_ctx,
            vec![],
        )
        .await
        .expect_err("query on nonexistent column must fail");
    assert_eq!(bad_err.sqlstate(), "42703");

    // Verify transaction is now in Aborted state (25P02)
    let tx_coordinator = request_transaction_coordinator(app_ctx.as_ref());
    let mut request_state = request_transaction_state(&req_ctx);
    request_state.sync(&tx_coordinator);
    let tx_id = request_state.active_transaction_id().cloned().expect("tx active");
    let handle = app_ctx.transaction_coordinator().get_handle(&tx_id).expect("tx handle");
    assert_eq!(handle.state, TransactionState::Aborted);

    // Subsequent ordinary statements must fail with 25P02
    let ignored_err = executor
        .execute(&insert_sql(&table_id, 4, "ignored"), &req_ctx, vec![])
        .await
        .expect_err("commands in aborted transaction block must be rejected");
    assert_eq!(ignored_err.sqlstate(), "25P02");

    // Recover using ROLLBACK TO SAVEPOINT sp1!
    execute_ok(&executor, &req_ctx, "ROLLBACK TO SAVEPOINT sp1").await;
    let handle_after = app_ctx.transaction_coordinator().get_handle(&tx_id).expect("tx handle");
    assert_eq!(handle_after.state, TransactionState::OpenWrite);

    // Row 2 was after sp1, so only row 1 remains
    assert_eq!(
        select_names(&executor, &req_ctx, &table_id).await,
        vec!["before_sp".to_string()]
    );

    execute_ok(&executor, &req_ctx, &insert_sql(&table_id, 5, "after_recovery")).await;
    execute_ok(&executor, &req_ctx, "COMMIT").await;

    assert_eq!(
        select_names(&executor, &observer_ctx, &table_id).await,
        vec!["before_sp".to_string(), "after_recovery".to_string()]
    );
}

#[tokio::test]
#[timeout(20000)]
async fn unsupported_isolation_levels_fail_with_0a000_and_invalid_savepoint_fails_with_3b001() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let req_ctx = request_exec_ctx(&app_ctx, "pg18-isolation-levels");

    for unsupported_sql in [
        "BEGIN ISOLATION LEVEL SERIALIZABLE",
        "START TRANSACTION ISOLATION LEVEL READ UNCOMMITTED",
        "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
    ] {
        let err = executor
            .execute(unsupported_sql, &req_ctx, vec![])
            .await
            .expect_err("unsupported isolation level must be rejected");
        assert_eq!(err.sqlstate(), "0A000", "SQL: {unsupported_sql}, err: {err}");
    }

    // Supported isolation levels succeed
    execute_ok(&executor, &req_ctx, "BEGIN ISOLATION LEVEL READ COMMITTED").await;
    execute_ok(&executor, &req_ctx, "SET TRANSACTION ISOLATION LEVEL SNAPSHOT").await;

    let sp_err = executor
        .execute("ROLLBACK TO SAVEPOINT missing_sp", &req_ctx, vec![])
        .await
        .expect_err("missing savepoint must fail");
    assert_eq!(sp_err.sqlstate(), "3B001");

    let rel_err = execute_err(&executor, &req_ctx, "RELEASE SAVEPOINT missing_sp").await;
    assert!(rel_err.contains("3B001") || rel_err.contains("does not exist"));

    execute_ok(&executor, &req_ctx, "ROLLBACK").await;
}
