mod support;

use std::sync::Arc;

use kalamdb_commons::{
    models::{SchemaId, UserId},
    Role,
};
use kalamdb_core::sql::{context::ExecutionResult, ExecutionContext};
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};
use ntest::timeout;
use support::{create_cluster_app_context, create_executor, execute_ok};

#[tokio::test]
#[timeout(20000)]
async fn subscribe_to_resolves_unqualified_table_using_active_session_schema() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let exec_ctx = ExecutionContext::with_namespace(
        UserId::new("root"),
        Role::System,
        SchemaId::new("default"),
        app_ctx.base_session_context(),
    )
    .with_request_id("pg18-sub-schema-req");

    execute_ok(&executor, &exec_ctx, "CREATE SCHEMA live_app").await;
    execute_ok(
        &executor,
        &exec_ctx,
        "CREATE TABLE live_app.feed (id BIGINT PRIMARY KEY, body TEXT NOT NULL) WITH (TYPE = 'SHARED')",
    )
    .await;

    // Switch session schema via SET search_path
    execute_ok(&executor, &exec_ctx, "SET search_path TO live_app").await;
    assert_eq!(exec_ctx.default_namespace().as_str(), "live_app");

    // Classify SUBSCRIBE TO unqualified table under live_app default schema
    let classified = SqlStatement::classify_and_parse(
        "SUBSCRIBE TO feed WHERE body = 'hello'",
        &exec_ctx.default_namespace(),
        Role::User,
    )
    .expect("SUBSCRIBE TO should classify for Role::User");
    match classified.kind() {
        SqlStatementKind::Subscribe(sub) => {
            assert_eq!(sub.namespace.as_str(), "live_app");
            assert_eq!(sub.table_name.as_str(), "feed");
            assert_eq!(
                sub.select_query,
                "SELECT * FROM live_app.feed WHERE body = 'hello'"
            );
        },
        other => panic!("expected Subscribe, got {other:?}"),
    }

    // Execute SUBSCRIBE TO through SqlExecutor
    let res = execute_ok(&executor, &exec_ctx, "SUBSCRIBE TO feed WHERE body = 'hello'").await;
    match res {
        ExecutionResult::Subscription {
            subscription_id,
            select_query,
            ..
        } => {
            assert!(subscription_id.starts_with("sub-live_app-feed-"));
            assert_eq!(
                select_query,
                "SELECT * FROM live_app.feed WHERE body = 'hello'"
            );
        },
        other => panic!("expected Subscription result, got {other:?}"),
    }
}
