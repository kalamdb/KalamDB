mod support;

use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::{
    models::{KalamCellValue, SessionOrigin, UserId},
    Role, TableAccess,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::sql::context::ExecutionContext;

use support::{
    create_cluster_app_context, create_cluster_app_context_with_config, create_executor,
    create_shared_table, create_shared_table_with_access, execute_err, execute_ok,
    observer_exec_ctx, result_rows, unique_namespace,
};

fn string_values(
    rows: &[std::collections::HashMap<String, KalamCellValue>],
    field: &str,
) -> Vec<String> {
    rows.iter()
        .filter_map(|row| row.get(field).and_then(|value| value.as_str()).map(ToString::to_string))
        .collect()
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shims_project_namespaces_tables_columns_and_database() {
    let mut config = ServerConfig::default();
    config.postgres_wire.pg_catalog_enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("pg_catalog_projection");
    let table_id = create_shared_table(&app_ctx, &namespace, "items").await;

    let namespace_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            format!(
                "SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname = '{}'",
                table_id.namespace_id()
            )
            .as_str(),
        )
        .await,
    );
    assert_eq!(
        string_values(&namespace_rows, "nspname"),
        vec![table_id.namespace_id().to_string()]
    );

    let class_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'items'",
        )
        .await,
    );
    assert_eq!(string_values(&class_rows, "relname"), vec!["items".to_string()]);

    let attribute_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT attname FROM pg_catalog.pg_attribute WHERE attname IN ('id', 'name') ORDER BY \
             attname",
        )
        .await,
    );
    assert_eq!(
        string_values(&attribute_rows, "attname"),
        vec!["id".to_string(), "name".to_string()]
    );

    let database_rows = result_rows(
        execute_ok(&executor, &observer_ctx, "SELECT datname FROM pg_catalog.pg_database").await,
    );
    assert_eq!(string_values(&database_rows, "datname"), vec!["kalam".to_string()]);
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_stat_activity_projects_backend_sessions() {
    let mut config = ServerConfig::default();
    config.postgres_wire.pg_catalog_enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let session_id = "019dabfa-1538-7c23-8e61-de751d8c1c38";

    app_ctx
        .backend_session_manager()
        .open_session(
            SessionOrigin::WireProtocol,
            session_id,
            BackendAuth::new(UserId::new("wire_user"), Role::Dba, "password", i64::MAX),
            Some("system".to_string()),
            Some("127.0.0.1:6543".to_string()),
        )
        .expect("open wire session");

    let rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT usename, backend_type FROM pg_catalog.pg_stat_activity WHERE usename = \
             'wire_user'",
        )
        .await,
    );

    assert_eq!(string_values(&rows, "usename"), vec!["wire_user".to_string()]);
    assert_eq!(string_values(&rows, "backend_type"), vec!["wire_protocol".to_string()]);
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_rbac_rejects_non_admin_stat_activity() {
    let mut config = ServerConfig::default();
    config.postgres_wire.pg_catalog_enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let user_ctx = ExecutionContext::new(
        UserId::new("basic_user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let error =
        execute_err(&executor, &user_ctx, "SELECT pid FROM pg_catalog.pg_stat_activity").await;
    assert!(
        error.contains("System tables require") || error.contains("Access denied"),
        "expected pg_stat_activity access failure, got: {error}"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_rbac_filters_private_table_metadata_for_non_admin() {
    let mut config = ServerConfig::default();
    config.postgres_wire.pg_catalog_enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let private_namespace = unique_namespace("pg_catalog_private");
    let private_table = create_shared_table_with_access(
        &app_ctx,
        &private_namespace,
        "private_items",
        TableAccess::Private,
    )
    .await;
    let user_ctx = ExecutionContext::new(
        UserId::new("basic_user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let namespace_rows = result_rows(
        execute_ok(
            &executor,
            &user_ctx,
            format!(
                "SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname = '{}'",
                private_table.namespace_id()
            )
            .as_str(),
        )
        .await,
    );
    assert!(namespace_rows.is_empty());

    let class_rows = result_rows(
        execute_ok(
            &executor,
            &user_ctx,
            "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'private_items'",
        )
        .await,
    );
    assert!(class_rows.is_empty());

    let attribute_rows = result_rows(
        execute_ok(
            &executor,
            &user_ctx,
            "SELECT a.attname FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON a.attrelid = c.oid \
             WHERE c.relname = 'private_items'",
        )
        .await,
    );
    assert!(attribute_rows.is_empty());
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_is_not_registered_when_disabled() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    let error =
        execute_err(&executor, &observer_ctx, "SELECT relname FROM pg_catalog.pg_class").await;
    assert!(
        error.contains("pg_catalog") || error.contains("not found"),
        "expected pg_catalog lookup failure, got: {error}"
    );
}
