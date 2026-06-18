//! Integration tests for shared table access control
//!
//! **Implements shared table access control tests**: Table sharing and access permissions
//!
//! These tests validate:
//! - Public table access across namespaces
//! - Private table access restrictions
//! - Restricted table access control
//! - Cross-user table permissions
//! - Table sharing functionality

use crate::common;

/// Test basic table creation and access
#[test]
fn test_basic_table_creation_and_access() {
    if !common::is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let table_name = common::generate_unique_table("shared_test");
    let namespace = "test_shared";

    // Create namespace first
    let _ = common::execute_sql_as_root_via_cli(&format!(
        "CREATE NAMESPACE IF NOT EXISTS {}",
        namespace
    ));

    // Create test table
    let create_sql = format!(
        r#"CREATE TABLE {}.{} (
            id BIGINT PRIMARY KEY AUTO_INCREMENT,
            content VARCHAR NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        ) WITH (TYPE='USER', FLUSH_POLICY='rows:10')"#,
        namespace, table_name
    );

    let result = common::execute_sql_as_root_via_cli(&create_sql);
    assert!(result.is_ok(), "Should create table successfully: {:?}", result.err());

    // Insert test data
    let insert_sql =
        format!("INSERT INTO {}.{} (content) VALUES ('test data')", namespace, table_name);
    let result = common::execute_sql_as_root_via_cli(&insert_sql);
    assert!(result.is_ok(), "Should insert data successfully: {:?}", result.err());

    // Query the data
    let select_sql = format!("SELECT content FROM {}.{}", namespace, table_name);
    let result = common::execute_sql_as_root_via_cli(&select_sql);
    assert!(result.is_ok(), "Should query data successfully: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("test data"), "Should contain inserted data: {}", output);

    // Cleanup
    let drop_sql = format!("DROP TABLE IF EXISTS {}.{}", namespace, table_name);
    let _ = common::execute_sql_as_root_via_cli(&drop_sql);
    let _ = common::execute_sql_as_root_via_cli(&format!(
        "DROP NAMESPACE IF EXISTS {} CASCADE",
        namespace
    ));
}

// Future tests to implement:
// - test_public_table_access
// - test_private_table_restrictions
// - test_restricted_table_permissions
// - test_cross_user_table_sharing
// - test_table_access_control_inheritance
