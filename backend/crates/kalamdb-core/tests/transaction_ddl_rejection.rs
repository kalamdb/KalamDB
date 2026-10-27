use crate::support::{execute_err, execute_ok, request_exec_ctx, setup_shared_table};

#[tokio::test]
#[ntest::timeout(30000)]
async fn sql_request_rejects_ddl_inside_explicit_transaction() {
    let (app_ctx, executor, table_id, _test_db) =
        setup_shared_table("sql_tx_ddl_reject", "items").await;
    let request_ctx = request_exec_ctx(&app_ctx, "sql-request-ddl-reject");
    let namespace = table_id.namespace_id().as_str();

    execute_ok(&executor, &request_ctx, "BEGIN").await;

    let error = execute_err(&executor, &request_ctx, "CREATE SCHEMA tx_ddl_should_fail").await;
    assert!(error.contains("DDL is not allowed inside explicit transaction"));

    execute_ok(&executor, &request_ctx, "ROLLBACK").await;

    execute_ok(&executor, &request_ctx, &format!("CREATE SCHEMA {namespace}")).await;
    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(
        &executor,
        &request_ctx,
        &format!(
            "CREATE TABLE {namespace}.tx_kept (id INT PRIMARY KEY, name TEXT) WITH (TYPE = \
             'SHARED')"
        ),
    )
    .await;
    execute_ok(&executor, &request_ctx, "ROLLBACK").await;
    execute_ok(
        &executor,
        &request_ctx,
        &format!("INSERT INTO {namespace}.tx_kept (id, name) VALUES (1, 'kept')"),
    )
    .await;

    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(
        &executor,
        &request_ctx,
        &format!("ALTER TABLE {namespace}.tx_kept ADD COLUMN note TEXT"),
    )
    .await;
    execute_ok(&executor, &request_ctx, "ROLLBACK").await;
    execute_ok(
        &executor,
        &request_ctx,
        &format!("UPDATE {namespace}.tx_kept SET note = 'set' WHERE id = 1"),
    )
    .await;

    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(&executor, &request_ctx, &format!("DROP TABLE {namespace}.tx_kept")).await;
    execute_ok(&executor, &request_ctx, "ROLLBACK").await;
    let missing = execute_err(
        &executor,
        &request_ctx,
        &format!("INSERT INTO {namespace}.tx_kept (id, name) VALUES (2, 'gone')"),
    )
    .await;
    assert!(
        missing.to_ascii_lowercase().contains("does not exist")
            || missing.to_ascii_lowercase().contains("not found"),
        "{missing}"
    );
}

#[tokio::test]
#[ntest::timeout(45000)]
async fn one_of_two_transactional_deletes_of_the_same_row_affects_zero_rows() {
    let (app_ctx, executor, table_id, _test_db) =
        setup_shared_table("sql_tx_delete_claim", "items").await;
    let namespace = table_id.namespace_id().as_str();
    let writer = request_exec_ctx(&app_ctx, "sql-delete-claim-writer");
    execute_ok(
        &executor,
        &writer,
        &format!("INSERT INTO {namespace}.items (id, name) VALUES (1, 'session')"),
    )
    .await;

    let left_ctx = request_exec_ctx(&app_ctx, "sql-delete-claim-left");
    let right_ctx = request_exec_ctx(&app_ctx, "sql-delete-claim-right");
    execute_ok(&executor, &left_ctx, "BEGIN").await;
    execute_ok(&executor, &right_ctx, "BEGIN").await;

    let delete_sql = format!("DELETE FROM {namespace}.items WHERE id = 1 AND name = 'session'");
    let (left_rows, right_rows) = tokio::join!(
        async {
            let rows = execute_ok(&executor, &left_ctx, &delete_sql).await.affected_rows();
            execute_ok(&executor, &left_ctx, "COMMIT").await;
            rows
        },
        async {
            let rows = execute_ok(&executor, &right_ctx, &delete_sql).await.affected_rows();
            execute_ok(&executor, &right_ctx, "COMMIT").await;
            rows
        },
    );

    let mut rows = [left_rows, right_rows];
    rows.sort_unstable();
    assert_eq!(rows, [0, 1]);
}
