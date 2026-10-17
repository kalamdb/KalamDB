//! Integration tests for SQL context functions: KDB_CURRENT_USER(), KDB_CURRENT_USER(),
//! KDB_CURRENT_ROLE()

use std::sync::Arc;

use datafusion::prelude::SessionContext;
use kalamdb_commons::{Role, UserId};
use kalamdb_core::sql::{context::ExecutionContext, datafusion_session::DataFusionSessionFactory};
use kalamdb_session::AuthSession;

fn create_test_session() -> Arc<SessionContext> {
    let factory =
        DataFusionSessionFactory::new().expect("Failed to create DataFusionSessionFactory");
    let session = factory.create_session();
    DataFusionSessionFactory::ensure_extended_functions(&session);
    Arc::new(session)
}

#[tokio::test]
async fn test_current_user_returns_user_id() {
    let user_id = UserId::new("u_alice");
    let role = Role::User;

    let auth_session = AuthSession::with_auth_details(
        user_id.clone(),
        role,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    // KDB_CURRENT_USER() now returns user_id
    let result = session.sql("SELECT KDB_CURRENT_USER() AS current_user").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();

    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 1);

    let column = batch.column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "u_alice");
}

#[tokio::test]
async fn test_current_user_id_with_dba_role() {
    let user_id = UserId::new("u_admin");
    let role = Role::Dba;

    let auth_session = AuthSession::with_auth_details(
        user_id.clone(),
        role,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_USER() AS user_id").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();

    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);

    let column = batch.column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "u_admin");
}

#[tokio::test]
async fn test_current_user_id_with_system_role() {
    let user_id = UserId::system();
    let role = Role::System;

    let exec_ctx = ExecutionContext::new(user_id.clone(), role, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_USER() AS user_id").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();

    assert_eq!(batches.len(), 1);
    let column = batches[0].column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "system");
}

#[tokio::test]
async fn test_current_user_id_with_service_role() {
    let user_id = UserId::new("u_service");
    let role = Role::Service;

    let exec_ctx = ExecutionContext::new(user_id.clone(), role, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_USER() AS user_id").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();

    assert_eq!(batches.len(), 1);
    let column = batches[0].column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "u_service");
}

#[tokio::test]
async fn test_current_role_user() {
    let user_id = UserId::new("u_regular");
    let role = Role::User;

    let auth_session = AuthSession::with_auth_details(
        user_id,
        role,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_ROLE() AS role").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();

    assert_eq!(batches.len(), 1);
    let column = batches[0].column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "user");
}

#[tokio::test]
async fn test_current_role_dba() {
    let user_id = UserId::new("u_admin");
    let role = Role::Dba;

    let exec_ctx = ExecutionContext::new(user_id, role, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_ROLE() AS role").await.unwrap();
    let batches = result.collect().await.unwrap();

    let column = batches[0].column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "dba");
}

#[tokio::test]
async fn test_current_role_system() {
    let user_id = UserId::system();
    let role = Role::System;

    let exec_ctx = ExecutionContext::new(user_id, role, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_ROLE() AS role").await.unwrap();
    let batches = result.collect().await.unwrap();

    let column = batches[0].column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "system");
}

#[tokio::test]
async fn test_current_role_service() {
    let user_id = UserId::new("u_service");
    let role = Role::Service;

    let exec_ctx = ExecutionContext::new(user_id, role, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_ROLE() AS role").await.unwrap();
    let batches = result.collect().await.unwrap();

    let column = batches[0].column(0);
    let string_array =
        column.as_any().downcast_ref::<datafusion::arrow::array::StringArray>().unwrap();
    assert_eq!(string_array.value(0), "service");
}

#[tokio::test]
async fn test_all_three_functions_together() {
    let user_id = UserId::new("u_testuser");
    let role = Role::Dba;

    let auth_session = AuthSession::with_auth_details(
        user_id,
        role,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session
        .sql(
            "SELECT KDB_CURRENT_USER() AS current_user, KDB_CURRENT_USER() AS user_id, \
             KDB_CURRENT_ROLE() AS role",
        )
        .await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();

    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 3);

    // current_user now returns user_id
    let current_user_col = batch.column(0);
    let current_user_array = current_user_col
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    assert_eq!(current_user_array.value(0), "u_testuser");

    let user_id_col = batch.column(1);
    let user_id_array = user_id_col
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    assert_eq!(user_id_array.value(0), "u_testuser");

    let role_col = batch.column(2);
    let role_array = role_col
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    assert_eq!(role_array.value(0), "dba");
}

#[tokio::test]
async fn test_context_function_execution_uses_rewritten_aliases() {
    let user_id = UserId::new("u_admin");
    let role = Role::Dba;
    let session_context = create_test_session();

    let auth_session = AuthSession::with_auth_details(
        user_id,
        role,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, session_context);
    let session = exec_ctx.create_session_with_user();

    let result = session
        .sql(
            "SELECT KDB_CURRENT_USER() AS current_user, KDB_CURRENT_USER() AS user_id, \
             KDB_CURRENT_ROLE() AS role",
        )
        .await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let batches = result.unwrap().collect().await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);

    let current_user_col = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    let user_id_col = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    let role_col = batches[0]
        .column(2)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();

    assert_eq!(current_user_col.value(0), "u_admin");
    assert_eq!(user_id_col.value(0), "u_admin");
    assert_eq!(role_col.value(0), "dba");
}

#[tokio::test]
async fn test_functions_in_where_clause() {
    let user_id = UserId::new("u_admin");
    let role = Role::Dba;

    let auth_session = AuthSession::with_auth_details(
        user_id,
        role,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session
        .sql("SELECT 1 WHERE KDB_CURRENT_USER() = 'u_admin' AND KDB_CURRENT_ROLE() = 'dba'")
        .await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
}

#[tokio::test]
async fn test_functions_with_no_arguments() {
    let user_id = UserId::new("u_test");
    let role = Role::User;

    let auth_session = AuthSession::with_auth_details(
        user_id,
        role,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT KDB_CURRENT_USER(), KDB_CURRENT_ROLE()").await;
    assert!(result.is_ok(), "Query should succeed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
}

#[tokio::test]
async fn test_current_schema_returns_default_namespace() {
    let auth_session = AuthSession::with_auth_details(
        UserId::new("u_wire"),
        Role::System,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT CURRENT_SCHEMA() AS schema").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let batches = result.unwrap().collect().await.unwrap();
    let schema_col = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    assert_eq!(schema_col.value(0), "default");
}

#[tokio::test]
async fn test_current_schema_respects_execution_namespace() {
    use kalamdb_commons::SchemaId;

    let exec_ctx = ExecutionContext::with_namespace(
        UserId::new("u_wire"),
        Role::System,
        SchemaId::new("tenant_app"),
        create_test_session(),
    )
    .with_request_id("wire:test");

    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT CURRENT_SCHEMA() AS schema").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let batches = result.unwrap().collect().await.unwrap();
    let schema_col = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    assert_eq!(schema_col.value(0), "tenant_app");
}

#[tokio::test]
async fn test_current_database_returns_catalog_name() {
    let auth_session = AuthSession::with_auth_details(
        UserId::new("u_wire"),
        Role::System,
        kalamdb_commons::models::ConnectionInfo::new(None),
        kalamdb_session::AuthMethod::Bearer,
    );

    let exec_ctx = ExecutionContext::from_session(auth_session, create_test_session());
    let session = exec_ctx.create_session_with_user();

    let result = session.sql("SELECT CURRENT_DATABASE() AS database").await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let batches = result.unwrap().collect().await.unwrap();
    let database_col = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .unwrap();
    assert_eq!(database_col.value(0), "kalam");
}

#[tokio::test]
async fn test_context_functions_resolve_via_native_datafusion_udf_aliases_without_text_rewrite() {
    use kalamdb_commons::SchemaId;
    use kalamdb_sql::parse_statement_for_datafusion;

    let exec_ctx = ExecutionContext::with_namespace(
        UserId::new("u_native_alias"),
        Role::Dba,
        SchemaId::new("app_ns"),
        create_test_session(),
    );
    let session = exec_ctx.create_session_with_user();

    let statement = parse_statement_for_datafusion(
        "SELECT CURRENT_USER AS cu, CURRENT_USER_ID() AS cuid, CURRENT_ROLE AS cr, CURRENT_ROLE() \
         AS cr_fn, CURRENT_SCHEMA AS cs, CURRENT_SCHEMA() AS cs_fn, CURRENT_DATABASE AS cd, \
         CURRENT_DATABASE() AS cd_fn",
    )
    .expect("parse statement for datafusion");

    let plan = session
        .state()
        .statement_to_plan(datafusion::sql::parser::Statement::Statement(Box::new(statement)))
        .await
        .expect("statement_to_plan should resolve native UDF aliases");
    let batches = session
        .execute_logical_plan(plan)
        .await
        .expect("execute plan")
        .collect()
        .await
        .expect("collect batches");

    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    let str_val = |col_idx: usize| -> String {
        batch
            .column(col_idx)
            .as_any()
            .downcast_ref::<datafusion::arrow::array::StringArray>()
            .unwrap()
            .value(0)
            .to_string()
    };

    assert_eq!(str_val(0), "u_native_alias");
    assert_eq!(str_val(1), "u_native_alias");
    assert_eq!(str_val(2), "dba");
    assert_eq!(str_val(3), "dba");
    assert_eq!(str_val(4), "app_ns");
    assert_eq!(str_val(5), "app_ns");
    assert_eq!(str_val(6), "kalam");
    assert_eq!(str_val(7), "kalam");
}
