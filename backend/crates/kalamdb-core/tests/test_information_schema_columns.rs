//! Test for information_schema.columns implementation
//!
//! This test verifies that the information_schema.columns table is properly
//! registered and can be queried via SQL.

use std::sync::Arc;

use kalamdb_commons::NodeId;
use kalamdb_configs::ServerConfig;
use kalamdb_core::app_context::AppContext;
use kalamdb_store::test_utils::TestDb;

/// Helper to create AppContext with temporary RocksDB for testing
async fn create_test_app_context() -> (Arc<AppContext>, TestDb) {
    let test_db = TestDb::with_system_tables().expect("Failed to create test database");
    let storage_base_path = test_db.storage_dir().expect("Failed to create storage directory");
    let backend = test_db.backend();
    let app_context = AppContext::create_isolated(
        backend,
        NodeId::new(1),
        storage_base_path.to_string_lossy().into_owned(),
        ServerConfig::default(),
    );

    (app_context, test_db)
}

#[tokio::test]
async fn test_information_schema_columns_query() {
    // Initialize AppContext (which registers information_schema.columns)
    let (app_ctx, _test_db) = create_test_app_context().await;

    // Get the base session context
    let session = app_ctx.base_session_context();

    // Query information_schema.columns
    let sql = "SELECT table_catalog, table_schema, table_name, column_name FROM \
               information_schema.columns WHERE table_name = 'jobs' ORDER BY ordinal_position \
               LIMIT 5";

    let result = session.sql(sql).await;

    // Should not return an error
    assert!(result.is_ok(), "Query failed with error: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.expect("Failed to collect batches");

    // Verify we got results
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert!(total_rows > 0, "Expected at least 1 row from information_schema.columns, got 0");

    println!("✅ information_schema.columns query succeeded with {} rows", total_rows);
}

#[tokio::test]
async fn test_information_schema_columns_shows_system_jobs() {
    // Initialize AppContext
    let (app_ctx, _temp_dir) = create_test_app_context().await;

    // Get the base session context
    let session = app_ctx.base_session_context();

    // Query for system.jobs columns specifically
    let sql = "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE \
               table_schema = 'system' AND table_name = 'jobs' ORDER BY ordinal_position";

    let result = session.sql(sql).await;
    assert!(result.is_ok(), "Query failed: {:?}", result.err());

    let df = result.unwrap();
    let batches = df.collect().await.expect("Failed to collect batches");

    // Should have at least the job_id, job_type, status columns
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert!(
        total_rows >= 3,
        "Expected at least 3 columns for system.jobs, got {}",
        total_rows
    );

    println!("✅ system.jobs has {} columns in information_schema.columns", total_rows);
}

#[tokio::test]
async fn test_information_schema_columns_includes_udt_name() {
    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();

    let sql = "SELECT column_name, data_type, udt_name FROM information_schema.columns WHERE \
               table_schema = 'system' AND table_name = 'jobs' ORDER BY ordinal_position";

    let batches = session
        .sql(sql)
        .await
        .expect("query should succeed")
        .collect()
        .await
        .expect("collect should succeed");

    let total_rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    assert!(total_rows >= 3, "expected columns for system.jobs");

    let batch = &batches[0];
    let udt_names = batch
        .column_by_name("udt_name")
        .expect("udt_name column")
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringArray>()
        .expect("string udt_name");
    for index in 0..batch.num_rows() {
        assert!(
            !udt_names.value(index).is_empty(),
            "udt_name must be populated for every column"
        );
    }
}

#[tokio::test]
async fn test_information_schema_columns_uses_sql_types_not_arrow() {
    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();

    let sql = "SELECT column_name, data_type, udt_name, kdb_data_type FROM \
               information_schema.columns WHERE table_schema = 'system' AND table_name = 'jobs' \
               AND column_name = 'node_id'";

    let batches = session
        .sql(sql)
        .await
        .expect("query should succeed")
        .collect()
        .await
        .expect("collect should succeed");

    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);

    let data_type = batch
        .column_by_name("data_type")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("data_type")
        .value(0);
    let udt_name = batch
        .column_by_name("udt_name")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("udt_name")
        .value(0);
    let kdb_data_type = batch
        .column_by_name("kdb_data_type")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("kdb_data_type")
        .value(0);

    assert_eq!(data_type, "bigint");
    assert_eq!(udt_name, "int8");
    assert_eq!(kdb_data_type, "BIGINT");
    assert!(!data_type.contains("Int"), "data_type must not expose Arrow names");
}

#[tokio::test]
async fn test_information_schema_columns_includes_kdb_metadata() {
    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();

    let sql = "SELECT column_name, kdb_namespace_id, kdb_version, kdb_column_id, kdb_primary_key, \
               kdb_primary_key_pos FROM information_schema.columns WHERE table_schema = 'system' \
               AND table_name = 'jobs' AND column_name = 'node_id'";

    let batches = session
        .sql(sql)
        .await
        .expect("query should succeed")
        .collect()
        .await
        .expect("collect should succeed");

    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);

    let namespace_id = batch
        .column_by_name("kdb_namespace_id")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("kdb_namespace_id")
        .value(0);
    let version = batch
        .column_by_name("kdb_version")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::Int64Array>())
        .expect("kdb_version")
        .value(0);
    let column_id = batch
        .column_by_name("kdb_column_id")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::Int64Array>())
        .expect("kdb_column_id")
        .value(0);
    let primary_key = batch
        .column_by_name("kdb_primary_key")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::BooleanArray>())
        .expect("kdb_primary_key")
        .value(0);

    assert_eq!(namespace_id, "system");
    assert!(version > 0);
    assert!(column_id > 0);
    assert!(!primary_key);
}

#[tokio::test]
async fn test_information_schema_tables_includes_kdb_metadata() {
    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();

    let sql = "SELECT table_name, kdb_namespace_id, kdb_table_type, kdb_version FROM \
               information_schema.tables WHERE table_schema = 'system' AND table_name = 'jobs'";

    let batches = session
        .sql(sql)
        .await
        .expect("query should succeed")
        .collect()
        .await
        .expect("collect should succeed");

    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);

    let table_name = batch
        .column_by_name("table_name")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("table_name")
        .value(0);
    let namespace_id = batch
        .column_by_name("kdb_namespace_id")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("kdb_namespace_id")
        .value(0);
    let table_type = batch
        .column_by_name("kdb_table_type")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("kdb_table_type")
        .value(0);
    let version = batch
        .column_by_name("kdb_version")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::Int64Array>())
        .expect("kdb_version")
        .value(0);

    assert_eq!(table_name, "jobs");
    assert_eq!(namespace_id, "system");
    assert_eq!(table_type, "system");
    assert!(version > 0);
}

#[tokio::test]
async fn test_dbeaver_column_metadata_query_plans() {
    use kalamdb_sql::rewrite_context_functions_for_datafusion;

    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();

    let sql = r#"
      SELECT
        table_schema,
        table_name,
        column_name,
        is_nullable,
        ordinal_position,
        column_default,
        CASE
          WHEN character_maximum_length is not null and udt_name != 'text'
            THEN udt_name || '(' || character_maximum_length::varchar(255) || ')'
          WHEN numeric_precision is not null and numeric_scale is not null
            THEN udt_name || '(' || numeric_precision::varchar(255) || ',' || numeric_scale::varchar(255) || ')'
          WHEN numeric_precision is not null and numeric_scale is null
            THEN udt_name || '(' || numeric_precision::varchar(255) || ')'
          WHEN datetime_precision is not null AND udt_name != 'date' THEN
            udt_name || '(' || datetime_precision::varchar(255) || ')'
          ELSE udt_name
        END as data_type,
        CASE
          WHEN data_type = 'ARRAY' THEN 'YES'
          ELSE 'NO'
        END as is_array,
        pg_catalog.col_description(format('%I.%I', table_schema, table_name)::regclass::oid, ordinal_position) as column_comment
      FROM information_schema.columns
      WHERE table_schema = 'system' AND table_name = 'jobs'
      ORDER BY table_schema, table_name, ordinal_position
    "#;

    let rewritten = rewrite_context_functions_for_datafusion(sql);
    let batches = session
        .sql(rewritten.as_ref())
        .await
        .expect("dbeaver column metadata query should plan")
        .collect()
        .await
        .expect("collect should succeed");

    let total_rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    assert!(total_rows >= 3, "expected columns for system.jobs");
}

#[tokio::test]
async fn test_dbeaver_column_metadata_query_with_bind_parameters() {
    use datafusion_common::ScalarValue;
    use kalamdb_core::sql::context::ExecutionContext;
    use kalamdb_sql::rewrite_context_functions_for_datafusion;

    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();
    let executor = kalamdb_core::sql::executor::SqlExecutor::new(
        app_ctx.clone(),
        std::sync::Arc::new(kalamdb_core::sql::executor::handler_registry::HandlerRegistry::new()),
    );
    let exec_ctx = ExecutionContext::with_namespace(
        kalamdb_commons::models::UserId::new("admin"),
        kalamdb_commons::Role::Dba,
        kalamdb_commons::NamespaceId::new("public"),
        session,
    );

    let sql = r#"
      SELECT column_name, ordinal_position,
        pg_catalog.col_description(format('%I.%I', table_schema, table_name)::regclass::oid, ordinal_position) as column_comment
      FROM information_schema.columns
      WHERE table_schema = $1 AND table_name = $2
      ORDER BY ordinal_position
    "#;
    let rewritten = rewrite_context_functions_for_datafusion(sql);
    let params = vec![
        ScalarValue::Utf8(Some("system".to_string())),
        ScalarValue::Utf8(Some("jobs".to_string())),
    ];

    executor
        .execute(rewritten.as_ref(), &exec_ctx, params)
        .await
        .expect("parameterized dbeaver column metadata query should execute");
}

#[tokio::test]
async fn test_information_schema_columns_fallback_kalam_types_for_arrow_names() {
    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();

    let sql = "SELECT column_name, data_type, kdb_data_type FROM information_schema.columns WHERE \
               table_schema = 'system' AND table_name = 'jobs' AND column_name = 'status'";

    let batches = session
        .sql(sql)
        .await
        .expect("query should succeed")
        .collect()
        .await
        .expect("collect should succeed");

    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);

    let data_type = batch
        .column_by_name("data_type")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("data_type")
        .value(0);
    let kdb_data_type = batch
        .column_by_name("kdb_data_type")
        .and_then(|array| array.as_any().downcast_ref::<datafusion::arrow::array::StringArray>())
        .expect("kdb_data_type")
        .value(0);

    assert_eq!(kdb_data_type, "TEXT");
    assert!(!kdb_data_type.contains("Utf8"));
    assert_ne!(data_type, "Utf8");
}
