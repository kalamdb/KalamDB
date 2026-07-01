//! Tests for extended `information_schema.parameters`.

use std::sync::Arc;

use kalamdb_commons::NodeId;
use kalamdb_configs::ServerConfig;
use kalamdb_core::app_context::AppContext;
use kalamdb_store::test_utils::TestDb;

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
async fn test_information_schema_parameters_includes_character_maximum_length() {
    let (app_ctx, _test_db) = create_test_app_context().await;
    let session = app_ctx.base_session_context();

    let sql = "\
      SELECT p.parameter_name, p.character_maximum_length
      FROM information_schema.routines r
      LEFT JOIN information_schema.parameters p
        ON p.specific_schema = r.routine_schema
        AND p.specific_name = r.specific_name
      WHERE p.parameter_mode = 'IN'
      LIMIT 1";

    let batches = session
        .sql(sql)
        .await
        .expect("parameters join query should plan")
        .collect()
        .await
        .expect("collect should succeed");

    let schema = batches
        .first()
        .map(|batch| batch.schema())
        .expect("expected schema from empty or populated result");
    assert!(
        schema.field_with_name("character_maximum_length").is_ok(),
        "parameters must expose character_maximum_length"
    );
}
