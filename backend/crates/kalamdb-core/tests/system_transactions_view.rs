mod support;

use std::{collections::HashMap, sync::Arc, time::Duration};

use datafusion_common::ScalarValue;
use kalamdb_auth::{create_and_sign_token, services::unified::init_auth_config};
use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::{
    conversions::arrow_json_conversion::record_batch_to_json_rows,
    models::{pg_operations::InsertRequest, rows::Row, KalamCellValue, TransactionId, UserId},
    Role as CommonsRole, TableType,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    operations::service::OperationService,
    sql::context::{ExecutionContext, ExecutionResult},
    transactions::ExecutionOwnerKey,
};
use kalamdb_pg::{
    BeginTransactionRequest, InsertRpcRequest, KalamPgService, OpenSessionRequest,
    OperationExecutor, PgService, RollbackTransactionRequest, ScanRpcRequest,
};
use kalamdb_raft::RaftExecutor;
use kalamdb_system::{providers::storages::models::StorageMode, AuthType, Role, User};
use support::{
    create_cluster_app_context, create_cluster_app_context_with_config, create_executor,
    create_shared_table, create_user_table, execute_err, execute_ok, insert_sql, observer_exec_ctx,
    request_exec_ctx, request_transaction_coordinator, request_transaction_state, row,
    unique_namespace,
};
use tonic::Request;

fn json_rows(result: ExecutionResult) -> Vec<HashMap<String, KalamCellValue>> {
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected row result");
    };

    let mut rows = Vec::new();
    for batch in batches {
        rows.extend(record_batch_to_json_rows(&batch).expect("json rows"));
    }
    rows
}

fn string_field(row: &HashMap<String, KalamCellValue>, field: &str) -> String {
    row.get(field)
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| panic!("missing string field {field}: {row:?}"))
        .to_string()
}

fn i64_field(row: &HashMap<String, KalamCellValue>, field: &str) -> i64 {
    row.get(field)
        .and_then(|value| {
            value.as_i64().or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
        })
        .unwrap_or_else(|| panic!("missing i64 field {field}: {row:?}"))
}

fn optional_string_field(row: &HashMap<String, KalamCellValue>, field: &str) -> Option<String> {
    row.get(field).and_then(|value| value.as_str()).map(ToString::to_string)
}

fn user_insert_request(
    table_id: &kalamdb_commons::TableId,
    session_id: &str,
    user_id: &str,
    id: i64,
    name: &str,
) -> InsertRpcRequest {
    let row_json = serde_json::to_string(&Row::new(std::collections::BTreeMap::from([
        ("id".to_string(), ScalarValue::Int64(Some(id))),
        ("name".to_string(), ScalarValue::Utf8(Some(name.to_string()))),
    ])))
    .expect("serialize typed row");

    InsertRpcRequest {
        namespace:  table_id.namespace_id().to_string(),
        table_name: table_id.table_name().to_string(),
        table_type: "user".to_string(),
        session_id: session_id.to_string(),
        user_id:    Some(user_id.to_string()),
        rows_json:  vec![row_json],
    }
}

async fn open_session(
    app_ctx: &Arc<AppContext>,
    service: &KalamPgService,
    session_label: &str,
) -> String {
    init_auth_config(&app_ctx.config().auth);

    let bridge_user_id = UserId::new(format!("{}_bridge_dba", session_label.replace('-', "_")));
    let bridge_user = User {
        user_id:               bridge_user_id.clone(),
        password_hash:         "$2b$12$unusedhashunusedhashunusedhashunusedhashunusedhashu"
            .to_string(),
        role:                  Role::Dba,
        name:                  None,
        email:                 Some(format!("{}@example.com", bridge_user_id.as_str())),
        auth_type:             AuthType::Password,
        auth_data:             None,
        storage_mode:          StorageMode::Table,
        storage_id:            None,
        failed_login_attempts: 0,
        locked_until:          None,
        last_login_at:         None,
        created_at:            0,
        updated_at:            0,
        last_seen:             None,
        deleted_at:            None,
        invite_expires_at:     None,
        invited_by:            None,
    };
    app_ctx
        .system_tables()
        .users()
        .create_user(bridge_user)
        .expect("create bridge dba user");

    let (token, _) = create_and_sign_token(
        &bridge_user_id,
        &Role::Dba,
        None,
        None,
        &app_ctx.config().auth.jwt_secret,
    )
    .expect("create bridge bearer token");

    let mut request = Request::new(OpenSessionRequest {
        session_id:     session_label.to_string(),
        current_schema: None,
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", token).parse().expect("valid auth metadata"),
    );

    service
        .open_session(request)
        .await
        .expect("open session succeeds")
        .into_inner()
        .session_id
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn system_sessions_lists_extension_and_wire_origins() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let executor_handle = app_ctx.executor();
    let raft_executor = executor_handle
        .as_any()
        .downcast_ref::<RaftExecutor>()
        .expect("raft executor available");
    let pg_service = raft_executor.pg_service().expect("pg service is wired");

    let extension_session_id = open_session(&app_ctx, &pg_service, "pg-777-feedface").await;
    app_ctx
        .backend_session_manager()
        .open_session(
            kalamdb_commons::models::SessionOrigin::WireProtocol,
            "019dabfa-1538-7c23-8e61-de751d8c1c38",
            BackendAuth::new(UserId::new("wire_user"), CommonsRole::User, "password", i64::MAX),
            Some("public".to_string()),
            Some("127.0.0.1:6543".to_string()),
        )
        .expect("open mock wire session");

    let rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT session_id, origin, backend_pid, authenticated_user_id FROM system.sessions \
             ORDER BY origin, session_id",
        )
        .await,
    );

    assert_eq!(rows.len(), 2);
    let extension_row = rows
        .iter()
        .find(|row| string_field(row, "session_id") == extension_session_id)
        .expect("extension row present");
    assert_eq!(string_field(extension_row, "origin"), "extension_bridge");
    assert_eq!(i64_field(extension_row, "backend_pid"), 777);
    assert_eq!(
        string_field(extension_row, "authenticated_user_id"),
        "pg_777_feedface_bridge_dba"
    );

    let wire_row = rows
        .iter()
        .find(|row| string_field(row, "origin") == "wire_protocol")
        .expect("wire row present");
    assert_eq!(string_field(wire_row, "session_id"), "019dabfa-1538-7c23-8e61-de751d8c1c38");
    assert_eq!(optional_string_field(wire_row, "backend_pid"), None);
    assert_eq!(string_field(wire_row, "authenticated_user_id"), "wire_user");
}

async fn begin_transaction(service: &KalamPgService, session_id: &str) -> String {
    service
        .begin_transaction(Request::new(BeginTransactionRequest {
            session_id: session_id.to_string(),
        }))
        .await
        .expect("begin transaction succeeds")
        .into_inner()
        .transaction_id
}

async fn rollback_transaction(service: &KalamPgService, session_id: &str, transaction_id: &str) {
    service
        .rollback_transaction(Request::new(RollbackTransactionRequest {
            session_id:     session_id.to_string(),
            transaction_id: transaction_id.to_string(),
        }))
        .await
        .expect("rollback transaction succeeds");
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn system_transactions_shows_active_pg_and_sql_transactions_while_sessions_remain_pg_only() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("system_transactions"), "items").await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let operation_service = OperationService::new(Arc::clone(&app_ctx));
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let request_ctx = request_exec_ctx(&app_ctx, "txn-view-request");
    let executor_handle = app_ctx.executor();
    let raft_executor = executor_handle
        .as_any()
        .downcast_ref::<RaftExecutor>()
        .expect("raft executor available");
    let pg_service = raft_executor.pg_service().expect("pg service is wired");

    let session_id = open_session(&app_ctx, &pg_service, "pg-4101-abcd1234").await;
    let pg_transaction_id = begin_transaction(&pg_service, &session_id).await;

    let pg_user_id = UserId::new("pg-txn-view-user");
    operation_service
        .execute_insert(InsertRequest {
            table_id:   table_id.clone(),
            table_type: TableType::User,
            session_id: Some(session_id.to_string()),
            user_id:    Some(pg_user_id),
            rows:       vec![row(1, "from-pg")],
        })
        .await
        .expect("pg-origin write stages successfully");

    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(&executor, &request_ctx, &insert_sql(&table_id, 2, "from-sql")).await;
    let sql_request_coordinator = request_transaction_coordinator(app_ctx.as_ref());
    let mut sql_request_state = request_transaction_state(&request_ctx);
    sql_request_state.sync(&sql_request_coordinator);
    let sql_transaction_id = sql_request_state
        .active_transaction_id()
        .expect("sql transaction id present")
        .to_string();

    let transaction_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT transaction_id, owner_id, origin, state, write_count FROM system.transactions \
             ORDER BY origin, transaction_id",
        )
        .await,
    );
    assert_eq!(transaction_rows.len(), 2);

    let origins = transaction_rows
        .iter()
        .map(|row| string_field(row, "origin"))
        .collect::<Vec<_>>();
    assert_eq!(origins, vec!["PgRpc".to_string(), "SqlBatch".to_string()]);

    assert!(transaction_rows.iter().any(|row| {
        string_field(row, "transaction_id") == pg_transaction_id
            && string_field(row, "owner_id") == session_id
            && string_field(row, "state") == "open_write"
            && i64_field(row, "write_count") == 1
    }));
    assert!(transaction_rows.iter().any(|row| {
        string_field(row, "transaction_id") == sql_transaction_id
            && string_field(row, "owner_id") == "sql-req-txn-view-request"
            && string_field(row, "state") == "open_write"
            && i64_field(row, "write_count") == 1
    }));

    let session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT session_id, transaction_id, transaction_state FROM system.sessions ORDER BY \
             session_id",
        )
        .await,
    );
    assert_eq!(session_rows.len(), 1);
    assert_eq!(string_field(&session_rows[0], "session_id"), session_id);
    assert_eq!(string_field(&session_rows[0], "transaction_id"), pg_transaction_id);
    assert_eq!(string_field(&session_rows[0], "transaction_state"), "active");

    rollback_transaction(&pg_service, &session_id, &pg_transaction_id).await;
    execute_ok(&executor, &request_ctx, "ROLLBACK").await;

    let cleared_rows = json_rows(
        execute_ok(&executor, &observer_ctx, "SELECT transaction_id FROM system.transactions")
            .await,
    );
    assert!(cleared_rows.is_empty());
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn sql_request_transactions_do_not_create_session_rows() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id =
        create_shared_table(&app_ctx, &unique_namespace("system_sessions_sql_request"), "items")
            .await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let request_ctx = request_exec_ctx(&app_ctx, "sessions-view-sql-request");
    let observer_ctx = observer_exec_ctx(&app_ctx);

    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(&executor, &request_ctx, &insert_sql(&table_id, 1, "committed")).await;
    execute_ok(&executor, &request_ctx, "COMMIT").await;

    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(&executor, &request_ctx, &insert_sql(&table_id, 2, "rolled-back")).await;
    execute_ok(&executor, &request_ctx, "ROLLBACK").await;

    let session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT session_id FROM system.sessions ORDER BY session_id",
        )
        .await,
    );
    assert!(session_rows.is_empty());
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn non_admin_cannot_read_system_sessions() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let user_ctx = ExecutionContext::new(
        UserId::new("regular_sessions_user"),
        CommonsRole::User,
        app_ctx.base_session_context(),
    );

    let error = execute_err(&executor, &user_ctx, "SELECT session_id FROM system.sessions").await;

    assert!(
        error.contains("permission")
            || error.contains("denied")
            || error.contains("not authorized")
            || error.contains("system"),
        "expected system.sessions access denial, got: {error}"
    );
}

#[tokio::test]
#[ntest::timeout(12000)]
async fn stale_idle_pg_sessions_drop_out_of_sessions_view() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let executor_handle = app_ctx.executor();
    let raft_executor = executor_handle
        .as_any()
        .downcast_ref::<RaftExecutor>()
        .expect("raft executor available");
    let pg_service = raft_executor.pg_service().expect("pg service is wired");

    let session_id = open_session(&app_ctx, &pg_service, "pg-4202-idlefade").await;

    let active_session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT session_id, state FROM system.sessions WHERE session_id = '{session_id}'"
            ),
        )
        .await,
    );
    assert_eq!(active_session_rows.len(), 1);
    assert_eq!(string_field(&active_session_rows[0], "session_id"), session_id);
    assert_eq!(string_field(&active_session_rows[0], "state"), "idle");

    tokio::time::sleep(Duration::from_millis(5_200)).await;

    let cleared_session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!("SELECT session_id FROM system.sessions WHERE session_id = '{session_id}'"),
        )
        .await,
    );
    assert!(cleared_session_rows.is_empty());
}

#[tokio::test]
#[ntest::timeout(12000)]
async fn pg_passive_timeout_hides_stale_transaction_fields_from_sessions_view() {
    let mut config = ServerConfig::default();
    config.transaction_timeout_secs = 1;

    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let executor_handle = app_ctx.executor();
    let raft_executor = executor_handle
        .as_any()
        .downcast_ref::<RaftExecutor>()
        .expect("raft executor available");
    let pg_service = raft_executor.pg_service().expect("pg service is wired");

    let session_id = open_session(&app_ctx, &pg_service, "pg-4300-feedcafe").await;
    let transaction_id = begin_transaction(&pg_service, &session_id).await;

    let active_session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT session_id, state, transaction_id, transaction_state FROM system.sessions \
                 WHERE session_id = '{session_id}'"
            ),
        )
        .await,
    );
    assert_eq!(active_session_rows.len(), 1);
    assert_eq!(string_field(&active_session_rows[0], "state"), "idle in transaction");
    assert_eq!(
        optional_string_field(&active_session_rows[0], "transaction_id"),
        Some(transaction_id.clone())
    );
    assert_eq!(
        optional_string_field(&active_session_rows[0], "transaction_state"),
        Some("active".to_string())
    );

    tokio::time::sleep(Duration::from_millis(2200)).await;

    let timed_out_session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT session_id, state, transaction_id, transaction_state FROM system.sessions \
                 WHERE session_id = '{session_id}'"
            ),
        )
        .await,
    );
    assert_eq!(timed_out_session_rows.len(), 1);
    assert_eq!(string_field(&timed_out_session_rows[0], "state"), "idle");
    assert_eq!(optional_string_field(&timed_out_session_rows[0], "transaction_id"), None);
    assert_eq!(optional_string_field(&timed_out_session_rows[0], "transaction_state"), None);

    let timed_out_transaction_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT transaction_id FROM system.transactions WHERE transaction_id = \
                 '{transaction_id}'"
            ),
        )
        .await,
    );
    assert!(timed_out_transaction_rows.is_empty());

    let replacement_transaction_id = begin_transaction(&pg_service, &session_id).await;
    assert_ne!(replacement_transaction_id, transaction_id);
    rollback_transaction(&pg_service, &session_id, &replacement_transaction_id).await;
}

#[tokio::test]
#[ntest::timeout(12000)]
async fn pg_timeout_after_write_clears_sessions_and_transactions_views() {
    let mut config = ServerConfig::default();
    config.transaction_timeout_secs = 1;

    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let table_id =
        create_user_table(&app_ctx, &unique_namespace("pg_timeout_write"), "items").await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let executor_handle = app_ctx.executor();
    let raft_executor = executor_handle
        .as_any()
        .downcast_ref::<RaftExecutor>()
        .expect("raft executor available");
    let pg_service = raft_executor.pg_service().expect("pg service is wired");

    let session_id = open_session(&app_ctx, &pg_service, "pg-4301-deadbeef").await;
    let transaction_id = begin_transaction(&pg_service, &session_id).await;

    pg_service
        .insert(Request::new(user_insert_request(
            &table_id,
            &session_id,
            "pg-timeout-write-user",
            1,
            "pending",
        )))
        .await
        .expect("initial staged write succeeds");

    let active_session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT session_id, state, transaction_id, transaction_state FROM system.sessions \
                 WHERE session_id = '{session_id}'"
            ),
        )
        .await,
    );
    assert_eq!(active_session_rows.len(), 1);
    assert_eq!(string_field(&active_session_rows[0], "state"), "idle in transaction");
    assert_eq!(
        optional_string_field(&active_session_rows[0], "transaction_id"),
        Some(transaction_id.clone())
    );
    assert_eq!(
        optional_string_field(&active_session_rows[0], "transaction_state"),
        Some("active".to_string())
    );

    let active_transaction_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT transaction_id, state, write_count FROM system.transactions WHERE \
                 transaction_id = '{transaction_id}'"
            ),
        )
        .await,
    );
    assert_eq!(active_transaction_rows.len(), 1);
    assert_eq!(string_field(&active_transaction_rows[0], "state"), "open_write");
    assert_eq!(i64_field(&active_transaction_rows[0], "write_count"), 1);

    tokio::time::sleep(Duration::from_millis(2200)).await;

    let timeout_error = pg_service
        .insert(Request::new(user_insert_request(
            &table_id,
            &session_id,
            "pg-timeout-write-user",
            2,
            "late",
        )))
        .await
        .expect_err("follow-up write should fail after timeout");
    assert_eq!(timeout_error.code(), tonic::Code::FailedPrecondition);
    assert!(timeout_error.message().contains("timed out"), "{timeout_error}");

    let owner_key = ExecutionOwnerKey::from_pg_session_id(session_id.as_str())
        .expect("pg owner key should parse");
    let parsed_transaction_id =
        TransactionId::try_new(transaction_id.clone()).expect("transaction id should parse");
    assert!(app_ctx.transaction_coordinator().active_for_owner(&owner_key).is_none());
    assert!(
        app_ctx.transaction_coordinator().get_handle(&parsed_transaction_id).is_none(),
        "timed out PG write should not leave a handle behind"
    );
    assert!(
        app_ctx.transaction_coordinator().get_overlay(&parsed_transaction_id).is_none(),
        "timed out PG write should not leave staged rows behind"
    );

    let cleared_session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT session_id, state, transaction_id, transaction_state FROM system.sessions \
                 WHERE session_id = '{session_id}'"
            ),
        )
        .await,
    );
    assert_eq!(cleared_session_rows.len(), 1);
    assert_eq!(string_field(&cleared_session_rows[0], "state"), "idle");
    assert_eq!(optional_string_field(&cleared_session_rows[0], "transaction_id"), None);
    assert_eq!(optional_string_field(&cleared_session_rows[0], "transaction_state"), None);

    let cleared_transaction_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT transaction_id FROM system.transactions WHERE transaction_id = \
                 '{transaction_id}'"
            ),
        )
        .await,
    );
    assert!(cleared_transaction_rows.is_empty());

    let replacement_transaction_id = begin_transaction(&pg_service, &session_id).await;
    assert_ne!(replacement_transaction_id, transaction_id);
    rollback_transaction(&pg_service, &session_id, &replacement_transaction_id).await;
}

#[tokio::test]
#[ntest::timeout(12000)]
async fn pg_timeout_after_read_clears_sessions_and_transactions_views() {
    let mut config = ServerConfig::default();
    config.transaction_timeout_secs = 1;

    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let table_id =
        create_shared_table(&app_ctx, &unique_namespace("pg_timeout_read"), "items").await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let executor_handle = app_ctx.executor();
    let raft_executor = executor_handle
        .as_any()
        .downcast_ref::<RaftExecutor>()
        .expect("raft executor available");
    let pg_service = raft_executor.pg_service().expect("pg service is wired");

    let session_id = open_session(&app_ctx, &pg_service, "pg-4302-cafef00d").await;
    let transaction_id = begin_transaction(&pg_service, &session_id).await;

    let active_transaction_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT transaction_id, state, write_count FROM system.transactions WHERE \
                 transaction_id = '{transaction_id}'"
            ),
        )
        .await,
    );
    assert_eq!(active_transaction_rows.len(), 1);
    assert_eq!(string_field(&active_transaction_rows[0], "state"), "open_read");
    assert_eq!(i64_field(&active_transaction_rows[0], "write_count"), 0);

    tokio::time::sleep(Duration::from_millis(2200)).await;

    let timeout_error = pg_service
        .scan(Request::new(ScanRpcRequest {
            namespace:  table_id.namespace_id().to_string(),
            table_name: table_id.table_name().to_string(),
            table_type: "shared".to_string(),
            session_id: session_id.to_string(),
            user_id:    Some("pg-timeout-read-user".to_string()),
            columns:    vec![],
            filters:    vec![],
            limit:      None,
        }))
        .await
        .expect_err("scan should fail after timeout");
    assert_eq!(timeout_error.code(), tonic::Code::FailedPrecondition);
    assert!(
        timeout_error.message().contains("timed_out")
            || timeout_error.message().contains("timed out"),
        "{timeout_error}"
    );

    let owner_key = ExecutionOwnerKey::from_pg_session_id(session_id.as_str())
        .expect("pg owner key should parse");
    let parsed_transaction_id =
        TransactionId::try_new(transaction_id.clone()).expect("transaction id should parse");
    assert!(app_ctx.transaction_coordinator().active_for_owner(&owner_key).is_none());
    assert!(
        app_ctx.transaction_coordinator().get_handle(&parsed_transaction_id).is_none(),
        "timed out PG read should not leave a handle behind"
    );

    let cleared_session_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT session_id, state, transaction_id, transaction_state FROM system.sessions \
                 WHERE session_id = '{session_id}'"
            ),
        )
        .await,
    );
    assert_eq!(cleared_session_rows.len(), 1);
    assert_eq!(string_field(&cleared_session_rows[0], "state"), "idle");
    assert_eq!(optional_string_field(&cleared_session_rows[0], "transaction_id"), None);
    assert_eq!(optional_string_field(&cleared_session_rows[0], "transaction_state"), None);

    let cleared_transaction_rows = json_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &format!(
                "SELECT transaction_id FROM system.transactions WHERE transaction_id = \
                 '{transaction_id}'"
            ),
        )
        .await,
    );
    assert!(cleared_transaction_rows.is_empty());

    let replacement_transaction_id = begin_transaction(&pg_service, &session_id).await;
    assert_ne!(replacement_transaction_id, transaction_id);
    rollback_transaction(&pg_service, &session_id, &replacement_transaction_id).await;
}
