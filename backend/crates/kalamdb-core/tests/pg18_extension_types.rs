use crate::support;

use std::sync::Arc;

use bytes::Bytes;
use datafusion::arrow::array::{Array, BinaryArray, StringArray};
use kalamdb_commons::{
    models::{RoutineId, TypeId, UserId},
    Role,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};
use kalamdb_store::test_utils::TestDb;

async fn setup_app() -> (Arc<AppContext>, SqlExecutor, TestDb) {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app, test_db) = support::create_cluster_app_context_with_config(config).await;
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app), false);
    let executor = SqlExecutor::new(Arc::clone(&app), registry);
    (app, executor, test_db)
}

fn root_ctx(app: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(
        UserId::new("u_pg18_extension_types"),
        Role::Dba,
        app.base_session_context(),
    )
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql);
}

async fn exec_err(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> String {
    let ctx = root_ctx(app);
    executor
        .execute(sql, &ctx, vec![])
        .await
        .expect_err(&format!("expected error for `{sql}`"))
        .to_string()
}

async fn query_strings(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> Vec<String> {
    let ctx = root_ctx(app);
    match executor.execute(sql, &ctx, vec![]).await.expect(sql) {
        ExecutionResult::Rows { batches, .. } => {
            let mut out = Vec::new();
            for batch in batches {
                let col = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("expected StringArray");
                for i in 0..col.len() {
                    if !col.is_null(i) {
                        out.push(col.value(i).to_string());
                    }
                }
            }
            out
        },
        other => panic!("expected rows for `{sql}`, got {other:?}"),
    }
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn builtin_extension_catalog_persists_and_blocks_ddl_mutation() {
    let (app, executor, _test_db) = setup_app().await;

    let catalog = app.system_tables().catalog_stores();
    let file_before = catalog
        .get_type(&TypeId::new("pg_catalog.file"))
        .unwrap()
        .expect("pg_catalog.file should exist");
    let emb_before = catalog
        .get_type(&TypeId::new("pg_catalog.embedding"))
        .unwrap()
        .expect("pg_catalog.embedding should exist");
    let put_before = catalog
        .get_routine(&RoutineId::new("pg_catalog.file_put"))
        .unwrap()
        .expect("pg_catalog.file_put should exist");

    // Re-running ensure_builtin_extension_catalog (as happens on server restart) preserves exact
    // records
    catalog.ensure_builtin_extension_catalog().unwrap();
    let file_after = catalog.get_type(&TypeId::new("pg_catalog.file")).unwrap().unwrap();
    let emb_after = catalog.get_type(&TypeId::new("pg_catalog.embedding")).unwrap().unwrap();
    let put_after = catalog.get_routine(&RoutineId::new("pg_catalog.file_put")).unwrap().unwrap();

    assert_eq!(file_before, file_after);
    assert_eq!(emb_before, emb_after);
    assert_eq!(put_before, put_after);

    // Visible in pg_catalog.pg_type and pg_catalog.pg_proc
    let pg_types = query_strings(
        &app,
        &executor,
        "SELECT typname FROM pg_catalog.pg_type WHERE typname IN ('file', 'embedding') ORDER BY \
         typname",
    )
    .await;
    assert_eq!(pg_types, vec!["embedding".to_string(), "file".to_string()]);

    let pg_procs = query_strings(
        &app,
        &executor,
        "SELECT proname FROM pg_catalog.pg_proc WHERE proname IN ('file_put', 'file_read', \
         'file_delete') ORDER BY proname",
    )
    .await;
    assert_eq!(
        pg_procs,
        vec![
            "file_delete".to_string(),
            "file_put".to_string(),
            "file_read".to_string()
        ]
    );

    // DROP / ALTER on built-in system types and routines must fail
    let drop_file = exec_err(&app, &executor, "DROP TYPE pg_catalog.file").await;
    assert!(
        drop_file.to_ascii_lowercase().contains("cannot drop")
            || drop_file.to_ascii_lowercase().contains("built-in")
            || drop_file.to_ascii_lowercase().contains("reserved"),
        "unexpected error: {drop_file}"
    );
    let alter_file =
        exec_err(&app, &executor, "ALTER TYPE pg_catalog.file ADD ATTRIBUTE extra text").await;
    assert!(
        alter_file.to_ascii_lowercase().contains("cannot alter")
            || alter_file.to_ascii_lowercase().contains("built-in")
            || alter_file.to_ascii_lowercase().contains("reserved"),
        "unexpected error: {alter_file}"
    );
    let drop_fn = exec_err(&app, &executor, "DROP FUNCTION pg_catalog.file_put").await;
    assert!(
        drop_fn.to_ascii_lowercase().contains("cannot drop")
            || drop_fn.to_ascii_lowercase().contains("built-in")
            || drop_fn.to_ascii_lowercase().contains("system")
            || drop_fn.to_ascii_lowercase().contains("reserved")
            || drop_fn.to_ascii_lowercase().contains("unsupported"),
        "unexpected error: {drop_fn}"
    );
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn file_functions_mint_read_delete_and_reject_forged_references() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA ext_files").await;
    exec(
        &app,
        &executor,
        "CREATE TABLE ext_files.docs (id INT PRIMARY KEY, doc FILE NOT NULL) WITH (TYPE = \
         'SHARED')",
    )
    .await;

    // Reject caller-constructed composite literal
    let forged_tuple = exec_err(
        &app,
        &executor,
        "INSERT INTO ext_files.docs (id, doc) VALUES (1, ('999', 'f0001', 'forged.txt', 5, \
         'text/plain', 'deadbeef', NULL))",
    )
    .await;
    assert!(
        forged_tuple.to_ascii_lowercase().contains("file_put")
            || forged_tuple.to_ascii_lowercase().contains("minted")
            || forged_tuple.to_ascii_lowercase().contains("file"),
        "unexpected forged tuple error: {forged_tuple}"
    );

    // Reject unminted JSON literal
    let forged_json = exec_err(
        &app,
        &executor,
        r#"INSERT INTO ext_files.docs (id, doc) VALUES (1, '{"id":"999999","sub":"f0001","name":"forged.txt","size":5,"mime":"text/plain","sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"}')"#,
    )
    .await;
    assert!(
        forged_json.to_ascii_lowercase().contains("minted")
            || forged_json.to_ascii_lowercase().contains("file_put"),
        "unexpected forged json error: {forged_json}"
    );

    // Stage multipart upload and mint via file_put
    app.file_storage_service()
        .stage_placeholder(
            "upload1",
            "hello.txt",
            Bytes::from_static(b"hello world"),
            Some("text/plain"),
        )
        .expect("stage placeholder");
    exec(
        &app,
        &executor,
        "INSERT INTO ext_files.docs (id, doc) VALUES (1, file_put('upload1'))",
    )
    .await;

    // Read back via file_read
    let ctx = root_ctx(&app);
    match executor
        .execute("SELECT file_read(doc) FROM ext_files.docs WHERE id = 1", &ctx, vec![])
        .await
        .expect("file_read should succeed")
    {
        ExecutionResult::Rows { batches, .. } => {
            let col = batches[0]
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("expected BinaryArray");
            assert_eq!(col.value(0), b"hello world");
        },
        other => panic!("expected rows, got {other:?}"),
    }

    // Delete via file_delete (returns void / null) and verify subsequent read fails
    let ctx = root_ctx(&app);
    match executor
        .execute("SELECT file_delete(doc) FROM ext_files.docs WHERE id = 1", &ctx, vec![])
        .await
        .expect("file_delete should succeed")
    {
        ExecutionResult::Rows { batches, .. } => {
            assert_eq!(batches[0].num_rows(), 1);
            assert!(batches[0].column(0).is_null(0));
        },
        other => panic!("expected rows, got {other:?}"),
    }
    let read_after_delete =
        exec_err(&app, &executor, "SELECT file_read(doc) FROM ext_files.docs WHERE id = 1").await;
    assert!(
        read_after_delete.to_ascii_lowercase().contains("not found")
            || read_after_delete.to_ascii_lowercase().contains("file"),
        "expected read after delete to fail, got: {read_after_delete}"
    );
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn embedding_enforces_dimension_bounds() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA ext_vec").await;
    exec(
        &app,
        &executor,
        "CREATE TABLE ext_vec.items (id INT PRIMARY KEY, vec EMBEDDING(3) NOT NULL) WITH (TYPE = \
         'SHARED')",
    )
    .await;

    let zero_dim = exec_err(
        &app,
        &executor,
        "CREATE TABLE ext_vec.bad0 (id INT PRIMARY KEY, vec EMBEDDING(0)) WITH (TYPE = 'SHARED')",
    )
    .await;
    assert!(
        zero_dim.to_ascii_lowercase().contains("dimension")
            || zero_dim.to_ascii_lowercase().contains("embedding"),
        "unexpected error for EMBEDDING(0): {zero_dim}"
    );

    let huge_dim = exec_err(
        &app,
        &executor,
        "CREATE TABLE ext_vec.bad_big (id INT PRIMARY KEY, vec EMBEDDING(8193)) WITH (TYPE = \
         'SHARED')",
    )
    .await;
    assert!(
        huge_dim.to_ascii_lowercase().contains("dimension")
            || huge_dim.to_ascii_lowercase().contains("embedding"),
        "unexpected error for EMBEDDING(8193): {huge_dim}"
    );
}
