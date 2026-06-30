mod support;

use std::sync::Arc;

use kalamdb_commons::{
    models::{
        datatypes::KalamDataType,
        pg_operations::InsertRequest,
        schemas::{ColumnDefinition, TableDefinition, TableOptions},
        TableId, TableName,
    },
    schemas::ColumnDefault,
    TableType,
};
use kalamdb_core::operations::service::OperationService;
use kalamdb_core::sql::executor::PreparedExecutionStatement;
use kalamdb_pg::OperationExecutor;
use support::{
    create_cluster_app_context, create_executor, execute_ok, request_exec_ctx, row, select_names,
    unique_namespace,
};

async fn create_stream_table(
    app_ctx: &Arc<kalamdb_core::app_context::AppContext>,
    namespace: &kalamdb_commons::models::NamespaceId,
    table_name: &str,
) -> TableId {
    let table_id = TableId::new(namespace.clone(), TableName::new(table_name));
    let id_col = ColumnDefinition::new(
        1,
        "id".to_string(),
        1,
        KalamDataType::BigInt,
        false,
        true,
        false,
        ColumnDefault::None,
        None,
    );
    let name_col = ColumnDefinition::simple(2, "name", 2, KalamDataType::Text);

    let mut table_def = TableDefinition::new(
        namespace.clone(),
        table_id.table_name().clone(),
        TableType::Stream,
        vec![id_col, name_col],
        TableOptions::stream(120),
        None,
    )
    .expect("create stream table definition");
    app_ctx
        .system_columns_service()
        .add_system_columns(&mut table_def)
        .expect("add system columns");

    app_ctx
        .schema_registry()
        .register_table(table_def)
        .expect("register stream table");

    table_id
}

#[tokio::test]
#[ntest::timeout(6000)]
async fn explicit_transaction_rejects_stream_table_writes() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let service = OperationService::new(Arc::clone(&app_ctx));
    let session_id = "pg-3101-feedbeef";
    let table_id = TableId::new(unique_namespace("tx_stream"), TableName::new("events"));

    let transaction_id = service
        .begin_transaction(session_id)
        .await
        .expect("begin transaction succeeds")
        .expect("transaction id returned");

    let error = service
        .execute_insert(InsertRequest {
            table_id,
            table_type: TableType::Stream,
            session_id: Some(session_id.to_string()),
            user_id: None,
            rows: vec![row(1, "event")],
        })
        .await
        .expect_err("stream-table write should be rejected");
    assert!(
        error
            .message()
            .contains("stream tables are not supported inside explicit transactions"),
        "{error}"
    );

    service
        .rollback_transaction(session_id, &transaction_id)
        .await
        .expect("rollback succeeds after stream-table rejection");
}

#[tokio::test]
#[ntest::timeout(6000)]
async fn explicit_transaction_rejects_stream_table_writes_with_missing_prepared_table_type() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let namespace = unique_namespace("tx_stream_sql");
    let table_id = create_stream_table(&app_ctx, &namespace, "events").await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let exec_ctx = request_exec_ctx(&app_ctx, "sql-stream-guard");

    execute_ok(&executor, &exec_ctx, "BEGIN").await;

    let insert_sql = format!(
        "INSERT INTO {}.{} (id, name) VALUES (1, 'event')",
        table_id.namespace_id(),
        table_id.table_name()
    );
    let prepared = executor
        .prepare_statement_metadata(&insert_sql, &exec_ctx)
        .expect("prepare stream insert metadata");
    let prepared_without_table_type = PreparedExecutionStatement::new(
        prepared.sql.clone(),
        prepared.table_id.clone(),
        None,
        prepared.classified_statement.clone(),
        prepared.track_slow_query,
        prepared.parsed_dml.clone(),
    );

    let error = executor
        .execute_with_metadata(&prepared_without_table_type, &exec_ctx, vec![])
        .await
        .expect_err(
            "stream-table write should still be rejected when prepared metadata omits table_type",
        );
    assert!(
        error
            .to_string()
            .contains("stream tables are not supported inside explicit transactions"),
        "{error}"
    );

    execute_ok(&executor, &exec_ctx, "ROLLBACK").await;
    assert!(
        select_names(&executor, &exec_ctx, &table_id).await.is_empty(),
        "rejected stream insert must not persist rows"
    );
}
