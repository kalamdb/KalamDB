mod support;

use datafusion_common::ScalarValue;
use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::{
    models::{KalamCellValue, SessionOrigin, UserId},
    NamespaceId, Role,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::sql::{context::ExecutionContext, ExecutionResult};
use support::{
    create_cluster_app_context, create_cluster_app_context_with_config, create_executor,
    create_shared_table, execute_err, execute_ok, execute_ok_with_params, observer_exec_ctx,
    result_rows, unique_namespace,
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
    config.postgres_wire.enabled = true;
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
            format!(
                "SELECT a.attname FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON \
                 a.attrelid = c.oid JOIN pg_catalog.pg_namespace n ON c.relnamespace = n.oid \
                 WHERE n.nspname = '{namespace}' AND c.relname = 'items' AND a.attname IN ('id', \
                 'name') ORDER BY a.attname"
            )
            .as_str(),
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

    let non_template_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT datname FROM pg_catalog.pg_database WHERE datistemplate = false ORDER BY \
             datname",
        )
        .await,
    );
    assert_eq!(string_values(&non_template_rows, "datname"), vec!["kalam".to_string()]);

    let text_param_rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            "SELECT datname FROM pg_catalog.pg_database WHERE datistemplate = $1 ORDER BY datname",
            vec![ScalarValue::Boolean(Some(false))],
        )
        .await,
    );
    assert_eq!(string_values(&text_param_rows, "datname"), vec!["kalam".to_string()]);

    let unqualified_param_rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            "SELECT datname FROM pg_database WHERE datistemplate = $1 ORDER BY datname",
            vec![ScalarValue::Boolean(Some(false))],
        )
        .await,
    );
    assert_eq!(string_values(&unqualified_param_rows, "datname"), vec!["kalam".to_string()]);

    let text_cast_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT datname::text FROM pg_database WHERE datistemplate = false ORDER BY datname",
        )
        .await,
    );
    assert_eq!(string_values(&text_cast_rows, "datname"), vec!["kalam".to_string()]);

    let allowconn_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT datname FROM pg_catalog.pg_database WHERE datallowconn AND NOT datistemplate \
             ORDER BY datname",
        )
        .await,
    );
    assert_eq!(string_values(&allowconn_rows, "datname"), vec!["kalam".to_string()]);
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_pg_type_lists_column_types_for_namespace() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("pg_type_shim");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let type_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            format!(
                "SELECT t.typname FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON \
                 n.oid = t.typnamespace WHERE n.nspname = '{namespace}' ORDER BY t.typname"
            )
            .as_str(),
        )
        .await,
    );
    let typnames = string_values(&type_rows, "typname");
    assert!(typnames.contains(&"int8".to_string()));
    assert!(typnames.contains(&"text".to_string()));
}

const DBEAVER_PG_TYPE_SQL: &str =
    "\
SELECT n.nspname as schema, t.typname as typename, t.oid::integer as typeid FROM pg_type t LEFT \
     JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace WHERE (t.typrelid = 0 OR (SELECT \
     c.relkind = 'c' FROM pg_catalog.pg_class c WHERE c.oid = t.typrelid)) AND n.nspname NOT IN \
     ('pg_catalog', 'information_schema') AND t.typname !~ '^_';";

const BEEKEEPER_COLUMNS_SQL: &str =
    "\
SELECT a.attname, a.attnum, a.attnotnull, a.atttypid, a.atttypmod, a.attidentity, a.attgenerated, \
     a.attisdropped, pg_catalog.format_type(a.atttypid, a.atttypmod) AS formatted_type, \
     pg_catalog.col_description(c.oid, a.attnum) AS column_comment, \
     pg_catalog.pg_get_expr(ad.adbin, ad.adrelid) AS column_default FROM pg_catalog.pg_class c \
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace JOIN pg_catalog.pg_attribute a ON \
     a.attrelid = c.oid LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = c.oid AND ad.adnum = \
     a.attnum LEFT JOIN pg_catalog.pg_description d ON d.objoid = c.oid AND d.objsubid = a.attnum \
     WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY \
     a.attnum";

const BEEKEEPER_INFORMATION_SCHEMA_COLUMNS_SQL: &str =
    "\
SELECT table_schema, table_name, column_name, is_nullable, is_generated, ordinal_position, \
     column_default, CASE WHEN character_maximum_length is not null and udt_name != 'text' THEN \
     udt_name || '(' || character_maximum_length::varchar(255) || ')' WHEN numeric_precision is \
     not null and numeric_scale is not null THEN udt_name || '(' || \
     numeric_precision::varchar(255) || ',' || numeric_scale::varchar(255) || ')' WHEN \
     numeric_precision is not null and numeric_scale is null THEN udt_name || '(' || \
     numeric_precision::varchar(255) || ')' WHEN datetime_precision is not null AND udt_name != \
     'date' THEN udt_name || '(' || datetime_precision::varchar(255) || ')' ELSE udt_name END as \
     data_type, udt_schema, CASE WHEN data_type = 'ARRAY' THEN 'YES' ELSE 'NO' END as is_array, \
     pg_catalog.col_description( format('%I.%I', table_schema, table_name)::regclass::oid, \
     ordinal_position ) as column_comment FROM information_schema.columns WHERE table_schema = $1 \
     AND table_name = $2 ORDER BY table_schema, table_name, ordinal_position";

const BEEKEEPER_LIST_TABLE_COLUMNS_BULK_SQL: &str =
    "\
SELECT table_schema, table_name, column_name, is_nullable, ordinal_position, column_default, CASE \
     WHEN character_maximum_length is not null and udt_name != 'text' THEN udt_name || '(' || \
     character_maximum_length::varchar(255) || ')' WHEN numeric_precision is not null and \
     numeric_scale is not null THEN udt_name || '(' || numeric_precision::varchar(255) || ',' || \
     numeric_scale::varchar(255) || ')' WHEN numeric_precision is not null and numeric_scale is \
     null THEN udt_name || '(' || numeric_precision::varchar(255) || ')' WHEN datetime_precision \
     is not null AND udt_name != 'date' THEN udt_name || '(' || datetime_precision::varchar(255) \
     || ')' ELSE udt_name END as data_type, udt_schema, CASE WHEN data_type = 'ARRAY' THEN 'YES' \
     ELSE 'NO' END as is_array, pg_catalog.col_description( format('%I.%I', table_schema, \
     table_name)::regclass::oid, ordinal_position ) as column_comment FROM \
     information_schema.columns ORDER BY table_schema, table_name, ordinal_position";

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_dbeaver_unqualified_pg_type_query() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("pg_type_dbeaver");
    create_shared_table(&app_ctx, &namespace, "items").await;
    let exec_ctx = observer_ctx.with_namespace_id(NamespaceId::new(namespace.as_str()));

    let rows = result_rows(execute_ok(&executor, &exec_ctx, DBEAVER_PG_TYPE_SQL).await);
    assert!(
        !rows.is_empty(),
        "DBeaver pg_type metadata query should return rows for user namespaces"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_beekeeper_column_metadata_query() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("beekeeper_columns");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            BEEKEEPER_COLUMNS_SQL,
            vec![
                ScalarValue::Utf8(Some(namespace.to_string())),
                ScalarValue::Utf8(Some("items".to_string())),
            ],
        )
        .await,
    );

    let columns = string_values(&rows, "attname");
    assert!(columns.contains(&"id".to_string()));
    assert!(columns.contains(&"name".to_string()));
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_beekeeper_bulk_table_columns_query() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    let result = execute_ok(&executor, &observer_ctx, BEEKEEPER_LIST_TABLE_COLUMNS_BULK_SQL).await;
    let ExecutionResult::Rows { row_count, .. } = result else {
        panic!("expected rows from beekeeper bulk column metadata query");
    };
    assert!(
        row_count > 0,
        "Beekeeper bulk listTableColumns(null) should return column metadata"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_beekeeper_information_schema_column_query() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("beekeeper_info_columns");
    create_shared_table(&app_ctx, &namespace, "items").await;

    execute_ok_with_params(
        &executor,
        &observer_ctx,
        BEEKEEPER_INFORMATION_SCHEMA_COLUMNS_SQL,
        vec![
            ScalarValue::Utf8(Some(namespace.to_string())),
            ScalarValue::Utf8(Some("items".to_string())),
        ],
    )
    .await;

    let rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            "SELECT column_name, udt_schema, is_generated FROM information_schema.columns WHERE \
             table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
            vec![
                ScalarValue::Utf8(Some(namespace.to_string())),
                ScalarValue::Utf8(Some("items".to_string())),
            ],
        )
        .await,
    );

    let columns = string_values(&rows, "column_name");
    assert!(columns.contains(&"id".to_string()));
    assert!(columns.contains(&"name".to_string()));
    assert!(
        string_values(&rows, "udt_schema").iter().all(|schema| schema == "pg_catalog"),
        "Beekeeper expects PostgreSQL type schema metadata"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_lists_user_relations_as_tables_not_views() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("beekeeper_relation_kind");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let table_rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            "SELECT tablename FROM pg_tables WHERE schemaname = $1 AND tablename = $2",
            vec![
                ScalarValue::Utf8(Some(namespace.to_string())),
                ScalarValue::Utf8(Some("items".to_string())),
            ],
        )
        .await,
    );
    assert_eq!(string_values(&table_rows, "tablename"), vec!["items".to_string()]);

    let view_rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            "SELECT viewname FROM pg_views WHERE schemaname = $1 AND viewname = $2",
            vec![
                ScalarValue::Utf8(Some(namespace.to_string())),
                ScalarValue::Utf8(Some("items".to_string())),
            ],
        )
        .await,
    );
    assert!(view_rows.is_empty(), "user tables must not be exposed through pg_views");

    let matview_rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            "SELECT matviewname FROM pg_matviews WHERE schemaname = $1 AND matviewname = $2",
            vec![
                ScalarValue::Utf8(Some(namespace.to_string())),
                ScalarValue::Utf8(Some("items".to_string())),
            ],
        )
        .await,
    );
    assert!(matview_rows.is_empty(), "user tables must not be exposed through pg_matviews");
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn information_schema_views_lists_system_views_not_tables() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    let view_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT table_schema, table_name FROM information_schema.views WHERE table_schema = \
             'system' AND table_name = 'cluster'",
        )
        .await,
    );
    assert_eq!(
        string_values(&view_rows, "table_name"),
        vec!["cluster".to_string()],
        "system.cluster is a view and must appear in information_schema.views"
    );

    let table_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT table_schema, table_name FROM information_schema.views WHERE table_schema = \
             'system' AND table_name = 'users'",
        )
        .await,
    );
    assert!(
        table_rows.is_empty(),
        "system.users is a table and must not be listed as a view"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_classifies_views_and_tables() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    let class_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT relname, relkind FROM pg_catalog.pg_class WHERE relname IN ('cluster', \
             'audit_log') ORDER BY relname",
        )
        .await,
    );
    let relnames = string_values(&class_rows, "relname");
    let relkinds = string_values(&class_rows, "relkind");
    assert_eq!(relnames, vec!["audit_log".to_string(), "cluster".to_string()]);
    assert_eq!(relkinds, vec!["r".to_string(), "v".to_string()]);

    let pg_view_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT viewname FROM pg_catalog.pg_views WHERE schemaname = 'system' AND viewname = \
             'cluster'",
        )
        .await,
    );
    assert_eq!(string_values(&pg_view_rows, "viewname"), vec!["cluster".to_string()]);

    let pg_table_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = 'system' AND tablename \
             IN ('cluster', 'audit_log') ORDER BY tablename",
        )
        .await,
    );
    assert_eq!(
        string_values(&pg_table_rows, "tablename"),
        vec!["audit_log".to_string()],
        "views must not appear in pg_tables"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_beekeeper_system_table_column_metadata() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    let rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            BEEKEEPER_COLUMNS_SQL,
            vec![
                ScalarValue::Utf8(Some("system".to_string())),
                ScalarValue::Utf8(Some("audit_log".to_string())),
            ],
        )
        .await,
    );

    let columns = string_values(&rows, "attname");
    assert!(columns.contains(&"target".to_string()));
    assert!(columns.contains(&"details".to_string()));

    let formatted_types = string_values(&rows, "formatted_type");
    assert!(
        formatted_types.iter().any(|value| value == "text"),
        "format_type should return concrete PostgreSQL type names, got {formatted_types:?}"
    );
    assert!(
        !formatted_types.iter().any(|value| value == "unknown"),
        "format_type should not return unknown for audit_log columns"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_reports_conservative_postgres_version() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    let rows =
        result_rows(execute_ok(&executor, &observer_ctx, "SELECT version() AS version").await);
    let versions = string_values(&rows, "version");
    assert_eq!(versions, vec!["PostgreSQL 9.6.0 compatible KalamDB".to_string()]);
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_beekeeper_empty_auxiliary_catalogs() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    execute_ok(&executor, &observer_ctx, "SELECT inhrelid, inhparent FROM pg_inherits").await;
    execute_ok(&executor, &observer_ctx, "SELECT enumtypid, enumlabel FROM pg_enum").await;
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn information_schema_lists_user_relation_once_as_base_table() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("beekeeper_info_schema");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let rows = result_rows(
        execute_ok_with_params(
            &executor,
            &observer_ctx,
            "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = $1 \
             AND table_name = $2 ORDER BY table_type",
            vec![
                ScalarValue::Utf8(Some(namespace.to_string())),
                ScalarValue::Utf8(Some("items".to_string())),
            ],
        )
        .await,
    );
    assert_eq!(string_values(&rows, "table_name"), vec!["items".to_string()]);

    let table_types = string_values(&rows, "table_type");
    assert_eq!(table_types.len(), 1);
    assert!(
        matches!(table_types[0].as_str(), "BASE TABLE" | "BASE"),
        "expected base table classification, got {table_types:?}"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_unqualified_pg_type_resolves_via_rewrite() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("pg_type_unqualified");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let type_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx.with_namespace_id(NamespaceId::new(namespace.as_str())),
            "SELECT typname FROM pg_type WHERE typrelid = 0 ORDER BY typname LIMIT 5",
        )
        .await,
    );
    assert!(
        !type_rows.is_empty(),
        "unqualified pg_type should resolve to pg_catalog.pg_type"
    );
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_stat_activity_projects_backend_sessions() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
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
    config.postgres_wire.enabled = true;
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
async fn pg_catalog_lists_shared_tables_for_non_admin_while_rls_hides_rows() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let private_namespace = unique_namespace("pg_catalog_private");
    let private_table = create_shared_table(&app_ctx, &private_namespace, "private_items").await;
    let user_ctx = ExecutionContext::new(
        UserId::new("basic_user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    // Shared tables are FORCE RLS, not ACCESS_LEVEL. pg_catalog may list the
    // relation while SELECT returns 0 rows until CREATE POLICY grants access.
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
    assert_eq!(
        string_values(&namespace_rows, "nspname"),
        vec![private_table.namespace_id().to_string()]
    );

    let class_rows = result_rows(
        execute_ok(
            &executor,
            &user_ctx,
            "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'private_items'",
        )
        .await,
    );
    assert_eq!(string_values(&class_rows, "relname"), vec!["private_items".to_string()]);

    let attribute_rows = result_rows(
        execute_ok(
            &executor,
            &user_ctx,
            "SELECT a.attname FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON \
             a.attrelid = c.oid WHERE c.relname = 'private_items' AND a.attname IN ('id', 'name') \
             ORDER BY a.attname",
        )
        .await,
    );
    assert_eq!(
        string_values(&attribute_rows, "attname"),
        vec!["id".to_string(), "name".to_string()]
    );

    let data_rows = result_rows(
        execute_ok(
            &executor,
            &user_ctx,
            &format!("SELECT id FROM {}.private_items", private_table.namespace_id()),
        )
        .await,
    );
    assert!(data_rows.is_empty());

    let system_class_rows = result_rows(
        execute_ok(
            &executor,
            &user_ctx,
            "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'sessions'",
        )
        .await,
    );
    assert!(system_class_rows.is_empty());
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

fn jdbc_get_tables_sql(schema: &str, table: &str) -> String {
    format!(
        "SELECT current_database() AS \"TABLE_CAT\", n.nspname AS \"TABLE_SCHEM\", c.relname AS \
         \"TABLE_NAME\", CASE n.nspname ~ '^pg_' OR n.nspname = 'information_schema' WHEN true \
         THEN CASE WHEN n.nspname = 'pg_catalog' OR n.nspname = 'information_schema' THEN CASE \
         c.relkind WHEN 'r' THEN 'SYSTEM TABLE' WHEN 'v' THEN 'SYSTEM VIEW' ELSE NULL END ELSE \
         CASE c.relkind WHEN 'r' THEN 'TEMPORARY TABLE' ELSE NULL END END WHEN false THEN CASE \
         c.relkind WHEN 'r' THEN 'TABLE' WHEN 'v' THEN 'VIEW' ELSE NULL END ELSE NULL END AS \
         \"TABLE_TYPE\", d.description AS \"REMARKS\" FROM pg_catalog.pg_namespace n, \
         pg_catalog.pg_class c LEFT JOIN pg_catalog.pg_description d ON (c.oid = d.objoid AND \
         d.objsubid = 0 and d.classoid = 'pg_class'::regclass) WHERE c.relnamespace = n.oid AND \
         n.nspname LIKE '{schema}' AND c.relname LIKE '{table}' AND (false OR ( c.relkind = 'r' \
         AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' )) ORDER BY \
         \"TABLE_TYPE\",\"TABLE_SCHEM\",\"TABLE_NAME\""
    )
}

fn jdbc_get_schemas_sql(schema: &str) -> String {
    format!(
        "SELECT nspname AS \"TABLE_SCHEM\", current_database() AS \"TABLE_CATALOG\" FROM \
         pg_catalog.pg_namespace WHERE nspname <> 'pg_toast' AND (nspname !~ '^pg_temp_' OR \
         nspname = (pg_catalog.current_schemas(true))[1]) AND (nspname !~ '^pg_toast_temp_' OR \
         nspname = replace((pg_catalog.current_schemas(true))[1], 'pg_temp_', 'pg_toast_temp_')) \
         AND nspname LIKE '{schema}' ORDER BY \"TABLE_SCHEM\""
    )
}

fn jdbc_get_columns_sql(schema: &str, table: &str) -> String {
    format!(
        "SELECT * FROM (SELECT current_database(), \
         n.nspname,c.relname,a.attname,a.atttypid,a.attnotnull OR (t.typtype = 'd' AND \
         t.typnotnull) AS attnotnull,a.atttypmod,a.attlen,t.typtypmod, row_number() OVER \
         (PARTITION BY a.attrelid ORDER BY a.attnum) AS attnum, nullif(a.attidentity, '') as \
         attidentity, nullif(a.attgenerated, '') as attgenerated, \
         pg_catalog.pg_get_expr(def.adbin, def.adrelid) AS \
         adsrc,dsc.description,t.typbasetype,t.typtype FROM pg_catalog.pg_namespace n JOIN \
         pg_catalog.pg_class c ON (c.relnamespace = n.oid) JOIN pg_catalog.pg_attribute a ON \
         (a.attrelid=c.oid) JOIN pg_catalog.pg_type t ON (a.atttypid = t.oid) LEFT JOIN \
         pg_catalog.pg_attrdef def ON (a.attrelid=def.adrelid AND a.attnum = def.adnum) LEFT JOIN \
         pg_catalog.pg_description dsc ON (c.oid=dsc.objoid AND a.attnum = dsc.objsubid) LEFT \
         JOIN pg_catalog.pg_class dc ON (dc.oid=dsc.classoid AND dc.relname='pg_class') LEFT JOIN \
         pg_catalog.pg_namespace dn ON (dc.relnamespace=dn.oid AND dn.nspname='pg_catalog') WHERE \
         c.relkind in ('r','p','v','f','m') and a.attnum > 0 AND NOT a.attisdropped AND n.nspname \
         LIKE '{schema}' AND c.relname LIKE '{table}') c WHERE true ORDER BY \
         nspname,c.relname,attnum"
    )
}

fn jdbc_get_primary_keys_sql(schema: &str, table: &str) -> String {
    format!(
        "SELECT result.TABLE_CAT AS \"TABLE_CAT\", result.TABLE_SCHEM AS \"TABLE_SCHEM\", \
         result.TABLE_NAME AS \"TABLE_NAME\", result.COLUMN_NAME AS \"COLUMN_NAME\", \
         result.KEY_SEQ AS \"KEY_SEQ\", result.PK_NAME AS \"PK_NAME\" FROM ( SELECT \
         current_database() AS TABLE_CAT, n.nspname AS TABLE_SCHEM, ct.relname AS TABLE_NAME, \
         a.attname AS COLUMN_NAME, (information_schema._pg_expandarray(i.indkey)).n AS KEY_SEQ, \
         ci.relname AS PK_NAME, information_schema._pg_expandarray(i.indkey) AS KEYS, a.attnum AS \
         A_ATTNUM, i.indnkeyatts as KEY_COUNT FROM pg_catalog.pg_class ct JOIN \
         pg_catalog.pg_attribute a ON (ct.oid = a.attrelid) JOIN pg_catalog.pg_namespace n ON \
         (ct.relnamespace = n.oid) JOIN pg_catalog.pg_index i ON ( a.attrelid = i.indrelid) JOIN \
         pg_catalog.pg_class ci ON (ci.oid = i.indexrelid) WHERE true AND n.nspname = '{schema}' \
         AND ct.relname = '{table}' AND i.indisprimary ) result where result.A_ATTNUM = \
         (result.KEYS).x AND result.KEY_SEQ <= KEY_COUNT ORDER BY result.table_name, \
         result.pk_name, result.key_seq"
    )
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_jdbc_get_tables_query() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("jdbc_get_tables");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let rows = result_rows(
        execute_ok(&executor, &observer_ctx, &jdbc_get_tables_sql(namespace.as_str(), "items"))
            .await,
    );
    assert_eq!(string_values(&rows, "TABLE_NAME"), vec!["items".to_string()]);
    assert_eq!(string_values(&rows, "TABLE_TYPE"), vec!["TABLE".to_string()]);
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_jdbc_get_schemas_and_catalogs() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("jdbc_get_schemas");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let schema_rows = result_rows(
        execute_ok(&executor, &observer_ctx, &jdbc_get_schemas_sql(namespace.as_str())).await,
    );
    assert_eq!(string_values(&schema_rows, "TABLE_SCHEM"), vec![namespace.to_string()]);

    let catalog_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT datname AS \"TABLE_CAT\" FROM pg_catalog.pg_database WHERE datallowconn = \
             true ORDER BY datname",
        )
        .await,
    );
    assert_eq!(string_values(&catalog_rows, "TABLE_CAT"), vec!["kalam".to_string()]);

    let name_type_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            "SELECT t.typlen FROM pg_catalog.pg_type t, pg_catalog.pg_namespace n WHERE \
             t.typnamespace=n.oid AND t.typname='name' AND n.nspname='pg_catalog'",
        )
        .await,
    );
    assert_eq!(name_type_rows.len(), 1);
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_jdbc_get_columns_and_primary_keys() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);
    let namespace = unique_namespace("jdbc_get_columns");
    create_shared_table(&app_ctx, &namespace, "items").await;

    let column_rows = result_rows(
        execute_ok(&executor, &observer_ctx, &jdbc_get_columns_sql(namespace.as_str(), "items"))
            .await,
    );
    let names = string_values(&column_rows, "attname");
    assert!(names.contains(&"id".to_string()), "getColumns missing id: {names:?}");
    assert!(names.contains(&"name".to_string()), "getColumns missing name: {names:?}");
    let unique = names.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), names.len(), "getColumns should not duplicate columns: {names:?}");

    let pk_rows = result_rows(
        execute_ok(
            &executor,
            &observer_ctx,
            &jdbc_get_primary_keys_sql(namespace.as_str(), "items"),
        )
        .await,
    );
    assert_eq!(string_values(&pk_rows, "COLUMN_NAME"), vec!["id".to_string()]);
}

fn jdbc_typeinfo_cache_sql() -> &'static str {
    "SELECT typinput='array_in'::regproc as is_array, typtype, typname, pg_type.oid FROM \
     pg_catalog.pg_type LEFT JOIN (select ns.oid as nspoid, ns.nspname, r.r from pg_namespace as \
     ns join ( select s.r, (current_schemas(false))[s.r] as nspname from generate_series(1, \
     array_upper(current_schemas(false), 1)) as s(r) ) as r using ( nspname ) ) as sp ON sp.nspoid \
     = typnamespace ORDER BY sp.r, pg_type.oid DESC"
}

fn jdbc_typeinfo_oid_sql() -> &'static str {
    "SELECT typinput='pg_catalog.array_in'::regproc as is_array, typtype, typname, pg_type.oid \
     FROM pg_catalog.pg_type LEFT JOIN (select ns.oid as nspoid, ns.nspname, r.r from pg_namespace \
     as ns join ( select s.r, (current_schemas(false))[s.r] as nspname from generate_series(1, \
     array_upper(current_schemas(false), 1)) as s(r) ) as r using ( nspname ) ) as sp ON sp.nspoid \
     = typnamespace WHERE pg_type.oid = 2950 ORDER BY sp.r, pg_type.oid DESC"
}

fn jdbc_sql_keywords_sql() -> &'static str {
    "select string_agg(word, ',') from pg_catalog.pg_get_keywords() where word <> ALL \
     ('{a,abs,absolute,select,table}'::text[])"
}

#[tokio::test]
#[ntest::timeout(10_000)]
async fn pg_catalog_shim_jdbc_typeinfo_cache_and_keywords() {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app_ctx, _test_db) = create_cluster_app_context_with_config(config).await;
    let executor = create_executor(app_ctx.clone());
    let observer_ctx = observer_exec_ctx(&app_ctx);

    let type_rows =
        result_rows(execute_ok(&executor, &observer_ctx, jdbc_typeinfo_cache_sql()).await);
    let type_names = string_values(&type_rows, "typname");
    assert!(
        type_names.iter().any(|name| name == "int4"),
        "TypeInfoCache missing int4: {type_names:?}"
    );
    assert!(
        type_names.iter().any(|name| name == "uuid"),
        "TypeInfoCache missing uuid: {type_names:?}"
    );
    assert!(
        type_rows.iter().all(|row| row.get("is_array").is_some()),
        "TypeInfoCache missing is_array"
    );

    let uuid_rows =
        result_rows(execute_ok(&executor, &observer_ctx, jdbc_typeinfo_oid_sql()).await);
    assert_eq!(string_values(&uuid_rows, "typname"), vec!["uuid".to_string()]);

    let keyword_rows =
        result_rows(execute_ok(&executor, &observer_ctx, jdbc_sql_keywords_sql()).await);
    assert_eq!(keyword_rows.len(), 1, "getSQLKeywords should return one aggregate row");
    let keywords = keyword_rows[0]
        .values()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        keywords.split(',').any(|word| word == "limit"),
        "getSQLKeywords missing postgres extra keyword limit: {keywords:?}"
    );
    assert!(
        !keywords.split(',').any(|word| word == "select"),
        "getSQLKeywords should exclude SQL:2003 keyword select: {keywords:?}"
    );
}
