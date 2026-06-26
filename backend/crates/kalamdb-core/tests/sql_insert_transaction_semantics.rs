mod support;

use std::collections::HashMap;

use datafusion_common::ScalarValue;
use kalamdb_commons::{
    models::{KalamCellValue, UserId},
    Role,
};
use kalamdb_core::sql::context::{ExecutionContext, ExecutionResult};
use kalamdb_tables::UserTableProvider;
use support::{
    create_cluster_app_context, create_context_files_table, create_executor, create_user_table,
    execute_ok, execute_ok_with_params, request_exec_ctx, result_rows, select_names,
    unique_namespace,
};

fn row_i64(row: &HashMap<String, KalamCellValue>, column_name: &str) -> i64 {
    row.get(column_name)
        .and_then(|value| {
            value.as_i64().or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or_else(|| panic!("expected integer column '{}'", column_name))
}

fn row_text<'a>(row: &'a HashMap<String, KalamCellValue>, column_name: &str) -> &'a str {
    row.get(column_name)
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| panic!("expected text column '{}'", column_name))
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn load_user_rows(
    app_ctx: &std::sync::Arc<kalamdb_core::app_context::AppContext>,
    table_id: &kalamdb_commons::models::TableId,
    user_id: &UserId,
    first_id: i64,
    second_id: i64,
) -> (kalamdb_tables::UserTableRow, kalamdb_tables::UserTableRow) {
    let provider_arc = app_ctx
        .schema_registry()
        .get_provider(table_id)
        .expect("user provider should be registered");
    let provider = (provider_arc.as_ref() as &dyn std::any::Any)
        .downcast_ref::<UserTableProvider>()
        .expect("provider should downcast to UserTableProvider");

    let (_, first_row) = provider
        .find_by_pk(user_id, &ScalarValue::Int64(Some(first_id)))
        .await
        .expect("lookup succeeds")
        .expect("first row exists");
    let (_, second_row) = provider
        .find_by_pk(user_id, &ScalarValue::Int64(Some(second_id)))
        .await
        .expect("lookup succeeds")
        .expect("second row exists");

    (first_row, second_row)
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn multi_row_insert_statement_uses_one_internal_commit() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let user_id = UserId::from("sql-insert-batch-user");
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("sql_insert_batch"), "items").await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx =
        ExecutionContext::new(user_id.clone(), Role::User, app_ctx.base_session_context());

    let sql = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha'), (2, 'beta')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let result = execute_ok(&executor, &exec_ctx, &sql).await;
    assert!(matches!(result, ExecutionResult::Inserted { rows_affected: 2 }));

    let (first_row, second_row) = load_user_rows(&app_ctx, &table_id, &user_id, 1, 2).await;
    assert_eq!(first_row._commit_seq, second_row._commit_seq);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn separate_insert_statements_commit_independently() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let user_id = UserId::from("sql-insert-separate-user");
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("sql_insert_separate"), "items").await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx =
        ExecutionContext::new(user_id.clone(), Role::User, app_ctx.base_session_context());

    let first_insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let second_insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (2, 'beta')",
        table_id.namespace_id(),
        table_id.table_name()
    );

    execute_ok(&executor, &exec_ctx, &first_insert).await;
    execute_ok(&executor, &exec_ctx, &second_insert).await;

    let (first_row, second_row) = load_user_rows(&app_ctx, &table_id, &user_id, 1, 2).await;
    assert_ne!(first_row._commit_seq, second_row._commit_seq);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn explicit_transaction_keeps_multiple_inserts_in_one_commit() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_user_table(&app_ctx, &unique_namespace("sql_insert_tx"), "items").await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = request_exec_ctx(&app_ctx, "sql-insert-explicit-tx");
    let user_id = exec_ctx.user_id().clone();

    execute_ok(&executor, &exec_ctx, "BEGIN").await;

    let first_insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let second_insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (2, 'beta')",
        table_id.namespace_id(),
        table_id.table_name()
    );

    execute_ok(&executor, &exec_ctx, &first_insert).await;
    execute_ok(&executor, &exec_ctx, &second_insert).await;
    execute_ok(&executor, &exec_ctx, "COMMIT").await;

    let (first_row, second_row) = load_user_rows(&app_ctx, &table_id, &user_id, 1, 2).await;
    assert_eq!(first_row._commit_seq, second_row._commit_seq);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn insert_on_conflict_do_update_set_updates_existing_primary_key() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("sql_insert_conflict_update"), "items").await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-conflict-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    execute_ok(&executor, &exec_ctx, &insert).await;

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'beta') \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let result = execute_ok(&executor, &exec_ctx, &upsert).await;

    assert!(matches!(result, ExecutionResult::Inserted { rows_affected: 1 }));
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["beta"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn insert_on_conflict_do_update_set_inserts_missing_primary_key() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("sql_insert_conflict_insert"), "items").await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-conflict-insert-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha') \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let result = execute_ok(&executor, &exec_ctx, &upsert).await;

    assert!(matches!(result, ExecutionResult::Inserted { rows_affected: 1 }));
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["alpha"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn insert_on_conflict_do_update_set_uses_existing_transaction() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("sql_insert_conflict_tx"), "items").await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = request_exec_ctx(&app_ctx, "sql-insert-conflict-explicit-tx");

    execute_ok(&executor, &exec_ctx, "BEGIN").await;

    let insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    execute_ok(&executor, &exec_ctx, &insert).await;

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'beta') \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let result = execute_ok(&executor, &exec_ctx, &upsert).await;
    assert!(matches!(result, ExecutionResult::Inserted { rows_affected: 1 }));

    execute_ok(&executor, &exec_ctx, "COMMIT").await;

    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["beta"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn insert_on_conflict_do_update_set_returning_returns_updated_row() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_user_table(
        &app_ctx,
        &unique_namespace("sql_insert_conflict_return_update"),
        "items",
    )
    .await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-conflict-return-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    execute_ok(&executor, &exec_ctx, &insert).await;

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'beta') \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id, name AS returned_name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let rows = result_rows(execute_ok(&executor, &exec_ctx, &upsert).await);

    assert_eq!(rows.len(), 1);
    assert_eq!(row_i64(&rows[0], "id"), 1);
    assert_eq!(row_text(&rows[0], "returned_name"), "beta");
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["beta"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn insert_on_conflict_do_update_set_returning_returns_inserted_row() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_user_table(
        &app_ctx,
        &unique_namespace("sql_insert_conflict_return_insert"),
        "items",
    )
    .await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-conflict-return-insert-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha') \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id, name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let rows = result_rows(execute_ok(&executor, &exec_ctx, &upsert).await);

    assert_eq!(rows.len(), 1);
    assert_eq!(row_i64(&rows[0], "id"), 1);
    assert_eq!(row_text(&rows[0], "name"), "alpha");
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["alpha"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn insert_on_conflict_do_update_set_returning_uses_existing_transaction() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("sql_insert_conflict_return_tx"), "items")
            .await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = request_exec_ctx(&app_ctx, "sql-insert-conflict-return-explicit-tx");

    execute_ok(&executor, &exec_ctx, "BEGIN").await;

    let insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    execute_ok(&executor, &exec_ctx, &insert).await;

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'beta') \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id, name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let rows = result_rows(execute_ok(&executor, &exec_ctx, &upsert).await);

    assert_eq!(rows.len(), 1);
    assert_eq!(row_i64(&rows[0], "id"), 1);
    assert_eq!(row_text(&rows[0], "name"), "beta");

    execute_ok(&executor, &exec_ctx, "COMMIT").await;

    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["beta"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn parameterized_insert_returning_returns_inserted_row() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("sql_insert_param_returning"), "items").await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-param-returning-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let sql = format!(
        "INSERT INTO {}.{} (id, name) VALUES ($1, $2) RETURNING id, name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let rows = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &sql,
            vec![
                ScalarValue::Int64(Some(1)),
                ScalarValue::Utf8(Some("alpha".to_string())),
            ],
        )
        .await,
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(row_i64(&rows[0], "id"), 1);
    assert_eq!(row_text(&rows[0], "name"), "alpha");
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["alpha"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn parameterized_insert_on_conflict_do_update_set_returning_returns_updated_row() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_user_table(
        &app_ctx,
        &unique_namespace("sql_insert_param_conflict_return_update"),
        "items",
    )
    .await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-param-conflict-return-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES ($1, $2)",
        table_id.namespace_id(),
        table_id.table_name()
    );
    execute_ok_with_params(
        &executor,
        &exec_ctx,
        &insert,
        vec![
            ScalarValue::Int64(Some(1)),
            ScalarValue::Utf8(Some("alpha".to_string())),
        ],
    )
    .await;

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES ($1, $2) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id, name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let rows = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &upsert,
            vec![
                ScalarValue::Int64(Some(1)),
                ScalarValue::Utf8(Some("beta".to_string())),
            ],
        )
        .await,
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(row_i64(&rows[0], "id"), 1);
    assert_eq!(row_text(&rows[0], "name"), "beta");
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["beta"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn parameterized_insert_on_conflict_do_nothing_returning_returns_zero_rows() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_user_table(
        &app_ctx,
        &unique_namespace("sql_insert_param_conflict_return_nothing"),
        "items",
    )
    .await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-param-conflict-return-nothing-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let insert = format!(
        "INSERT INTO {}.{} (id, name) VALUES ($1, $2)",
        table_id.namespace_id(),
        table_id.table_name()
    );
    execute_ok_with_params(
        &executor,
        &exec_ctx,
        &insert,
        vec![
            ScalarValue::Int64(Some(1)),
            ScalarValue::Utf8(Some("alpha".to_string())),
        ],
    )
    .await;

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES ($1, $2) \
         ON CONFLICT (id) DO NOTHING \
         RETURNING id, name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let result = execute_ok_with_params(
        &executor,
        &exec_ctx,
        &upsert,
        vec![
            ScalarValue::Int64(Some(1)),
            ScalarValue::Utf8(Some("ignored".to_string())),
        ],
    )
    .await;

    assert!(matches!(result, ExecutionResult::Rows { row_count: 0, .. }));
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["alpha"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn insert_on_conflict_do_nothing_returning_skips_duplicate_key_from_same_statement() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_user_table(
        &app_ctx,
        &unique_namespace("sql_insert_conflict_same_statement_duplicate"),
        "items",
    )
    .await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-conflict-same-statement-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let upsert = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'alpha'), (1, 'ignored') \
         ON CONFLICT (id) DO NOTHING \
         RETURNING id, name",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let rows = result_rows(execute_ok(&executor, &exec_ctx, &upsert).await);

    assert_eq!(rows.len(), 1);
    assert_eq!(row_i64(&rows[0], "id"), 1);
    assert_eq!(row_text(&rows[0], "name"), "alpha");
    assert_eq!(select_names(&executor, &exec_ctx, &table_id).await, vec!["alpha"]);
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn parameterized_file_insert_on_conflict_supports_explicit_default_and_set_params() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_context_files_table(
        &app_ctx,
        &unique_namespace("sql_insert_context_files_conflict"),
        "context_files",
    )
    .await;
    let executor = create_executor(app_ctx.clone());
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-insert-context-file-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let first_file_ref = r#"{"id":"328909262921138176","sub":"f0001","name":"index.md","size":241,"mime":"text/markdown","sha256":"7505522b895796b845b734eab8baef4a60b6fe224cc2615978ee26a768bdb392"}"#;
    let second_file_ref = r#"{"id":"328909262921138177","sub":"f0001","name":"index.md","size":242,"mime":"text/markdown","sha256":"8505522b895796b845b734eab8baef4a60b6fe224cc2615978ee26a768bdb393"}"#;

    let upsert_sql = |file_ref: &str| {
        format!(
            "INSERT INTO {}.{} (path, file_ref, created_at, updated_at) \
             VALUES ($1, {}, DEFAULT, $2) \
             ON CONFLICT (path) DO UPDATE SET file_ref = {}, updated_at = $3 \
             RETURNING path, file_ref",
            table_id.namespace_id(),
            table_id.table_name(),
            sql_string(file_ref),
            sql_string(file_ref)
        )
    };

    let first_returned_rows = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &upsert_sql(first_file_ref),
            vec![
                ScalarValue::Utf8(Some("index.md".to_string())),
                ScalarValue::TimestampMicrosecond(Some(1_786_000_000_000_000), None),
                ScalarValue::TimestampMicrosecond(Some(1_786_000_000_000_001), None),
            ],
        )
        .await,
    );
    assert_eq!(first_returned_rows.len(), 1);
    assert_eq!(row_text(&first_returned_rows[0], "path"), "index.md");
    assert_eq!(row_text(&first_returned_rows[0], "file_ref"), first_file_ref);

    let second_returned_rows = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &upsert_sql(second_file_ref),
            vec![
                ScalarValue::Utf8(Some("index.md".to_string())),
                ScalarValue::TimestampMicrosecond(Some(1_786_000_000_000_010), None),
                ScalarValue::TimestampMicrosecond(Some(1_786_000_000_000_011), None),
            ],
        )
        .await,
    );
    assert_eq!(second_returned_rows.len(), 1);
    assert_eq!(row_text(&second_returned_rows[0], "path"), "index.md");
    assert_eq!(row_text(&second_returned_rows[0], "file_ref"), second_file_ref);

    let result = execute_ok(
        &executor,
        &exec_ctx,
        &format!(
            "SELECT path, file_ref FROM {}.{} WHERE path = 'index.md'",
            table_id.namespace_id(),
            table_id.table_name()
        ),
    )
    .await;
    let rows = result_rows(result);

    assert_eq!(rows.len(), 1);
    assert_eq!(row_text(&rows[0], "path"), "index.md");
    assert_eq!(row_text(&rows[0], "file_ref"), second_file_ref);
}
