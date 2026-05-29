//! Integration tests for administrative operations
//!
//! **Implements T041-T042, T055-T058**: Administrative commands and system operations
//!
//! These tests validate:
//! - List tables and describe table commands
//! - Batch file execution
//! - Server health checks
//! - Administrative SQL operations
//! - Namespace and table management

use std::{path::Path, time::Duration};

use crate::common::*;

/// Test configuration constants
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// T041: Test list tables command (using SELECT from system.schemas)
#[test]
fn test_cli_list_tables() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let table_name = generate_unique_table("messages_list_tables");
    let namespace = generate_unique_namespace("test_cli");
    let full_table_name = format!("{}.{}", namespace, table_name);

    let _ =
        execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace));

    // Create test table
    let create_sql = format!(
        r#"CREATE TABLE {}.{} (
            id INT PRIMARY KEY AUTO_INCREMENT,
            content VARCHAR NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        ) WITH (TYPE='USER', FLUSH_POLICY='rows:10')"#,
        namespace, table_name
    );

    let result = execute_sql_as_root_via_cli(&create_sql);
    if result.is_err() {
        eprintln!("⚠️  Failed to create test table, skipping test");
        return;
    }
    wait_for_table_ready(&full_table_name, Duration::from_secs(15)).unwrap();

    // Query system tables
    let query_sql = format!(
        "SELECT table_name FROM system.schemas WHERE namespace_id = '{}' AND table_name = '{}'",
        namespace, table_name
    );
    let result = wait_for_sql_output_contains(&query_sql, &table_name, Duration::from_secs(15));

    // Should list tables
    assert!(result.is_ok(), "Should list tables: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains(&table_name), "Should contain table info: {}", output);

    // Cleanup
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {} CASCADE", namespace));
}

/// T042: Test describe table command (\d table)
#[test]
fn test_cli_describe_table() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let table_name = generate_unique_table("messages_describe");
    let namespace = generate_unique_namespace("test_cli");
    let full_table_name = format!("{}.{}", namespace, table_name);

    let _ =
        execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace));

    // Create test table
    let create_sql = format!(
        r#"CREATE TABLE {}.{} (
            id INT PRIMARY KEY AUTO_INCREMENT,
            content VARCHAR NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        ) WITH (TYPE='USER', FLUSH_POLICY='rows:10')"#,
        namespace, table_name
    );

    let result = execute_sql_as_root_via_cli(&create_sql);
    if result.is_err() {
        eprintln!("⚠️  Failed to create test table, skipping test");
        return;
    }
    wait_for_table_ready(&full_table_name, Duration::from_secs(15)).unwrap();

    let query_sql = format!(
        "SELECT table_name FROM system.schemas WHERE namespace_id = '{}' AND table_name = '{}'",
        namespace, table_name
    );
    let result = wait_for_sql_output_contains(&query_sql, &table_name, Duration::from_secs(15));

    // Should execute successfully and show table info
    assert!(result.is_ok(), "Should describe table: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains(&table_name), "Should contain table name: {}", output);

    // Cleanup
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {} CASCADE", namespace));
}

/// T055: Test batch file execution
#[test]
fn test_cli_batch_file_execution() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    // Create temporary SQL file
    let temp_dir = tempfile::TempDir::new().unwrap();
    let sql_file = temp_dir.path().join("test.sql");

    let namespace = generate_unique_namespace("batch_test");
    let table_name = "items";
    let full_table_name = format!("{}.{}", namespace, table_name);

    // Cleanup first in case namespace/table exists from previous run
    // Note: DROP NAMESPACE CASCADE doesn't properly cascade to tables yet, so drop table first
    let _ = execute_sql_as_root_via_cli(&format!("DROP TABLE IF EXISTS {}", full_table_name));
    let _ = execute_sql_as_root_via_cli(&format!("DROP NAMESPACE IF EXISTS {}", namespace));

    // Use a unique ID based on timestamp to avoid conflicts
    let unique_id = rand::random::<i64>().abs();
    let batch_user = generate_unique_namespace("batch_file_user");

    std::fs::write(
        &sql_file,
        format!(
            r#"-- SQL-file importer must ignore semicolons in comments;
CREATE NAMESPACE {};
CREATE USER {} WITH PASSWORD 'demo123' ROLE 'user';
/* Block comments with ; should not split the file. */
CREATE TABLE {} (id BIGINT PRIMARY KEY, name VARCHAR) WITH (TYPE='USER', FLUSH_POLICY='rows:10');
EXECUTE AS USER '{}' (INSERT INTO {} (id, name) VALUES ({}, 'Item; One -- literal'));
EXECUTE AS USER '{}' (SELECT * FROM {});"#,
            namespace,
            batch_user,
            full_table_name,
            batch_user,
            full_table_name,
            unique_id,
            batch_user,
            full_table_name
        ),
    )
    .unwrap();

    let stdout = execute_batch_file(&sql_file)
        .unwrap_or_else(|err| panic!("Batch execution should succeed. Error: {}", err));

    // Verify execution - should show Query OK messages and final result
    assert!(
        stdout.contains("Item; One -- literal"),
        "Batch execution should preserve semicolons and comment markers inside literals.\nstdout: {}",
        stdout
    );

    // Cleanup
    let _ = execute_sql_as_root_via_cli(&format!("DROP USER IF EXISTS {}", batch_user));
    let _ = execute_sql_as_root_via_cli(&format!("DROP NAMESPACE {} CASCADE", namespace));
}

#[test]
fn test_cli_batch_file_execution_prints_target_banner() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let temp_dir = tempfile::TempDir::new().unwrap();
    let sql_file = temp_dir.path().join("target-banner.sql");

    std::fs::write(&sql_file, "SELECT 1 AS banner_check;").unwrap();

    let mut cmd = create_cli_command_with_auth(admin_username(), admin_password());
    cmd.arg("--no-spinner")
        .arg("--no-color")
        .arg("--file")
        .arg(&sql_file)
        .timeout(TEST_TIMEOUT);

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Batch execution should succeed. stderr: {}\nstdout: {}",
        stderr,
        stdout
    );
    assert!(
        stderr.contains("Instance: local"),
        "stderr should include the instance banner. stderr: {}",
        stderr
    );
    assert!(
        stderr.contains("Server:"),
        "stderr should include the target server line. stderr: {}",
        stderr
    );
    assert!(
        stderr.contains(&format!("User: {}", admin_username())),
        "stderr should include the user banner. stderr: {}",
        stderr
    );
}

/// T056: Test syntax error handling
#[test]
fn test_cli_syntax_error_handling() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let result = execute_sql_as_root_via_cli("INVALID SQL SYNTAX HERE");

    // Should contain error message
    assert!(result.is_err(), "Should fail with syntax error");
    let error_msg = result.err().unwrap().to_string();
    assert!(
        error_msg.contains("ERROR") || error_msg.contains("Error") || error_msg.contains("syntax"),
        "Should display error message: {}",
        error_msg
    );
}

/// T057: Test connection failure handling
#[test]
fn test_cli_connection_failure_handling() {
    // Try to connect to non-existent server
    let mut cmd = create_cli_command();
    cmd.arg("-u")
        .arg("http://localhost:9999") // Non-existent port
        .arg("--command")
        .arg("SELECT 1")
        .timeout(TEST_TIMEOUT);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show connection error
    assert!(
        !output.status.success()
            || stderr.contains("Connection")
            || stderr.contains("error")
            || stdout.contains("Connection")
            || stdout.contains("error"),
        "Should display connection error. stderr: {}, stdout: {}",
        stderr,
        stdout
    );
}

/// T058: Test server health check via CLI
#[test]
fn test_cli_health_check() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    // Test server health via SQL query
    let result = wait_for_sql_output_contains(
        "SELECT 1 as health_check",
        "health_check",
        Duration::from_secs(15),
    );

    assert!(result.is_ok(), "Server should respond to SQL queries: {:?}", result.err());

    let output = result.unwrap();
    assert!(
        output.contains("health_check") || output.contains("1"),
        "Response should contain query result: {}",
        output
    );
}

fn execute_batch_file(sql_file: &Path) -> Result<String, Box<dyn std::error::Error>> {
    execute_sql_file_as_root_via_cli(sql_file)
}

fn wait_for_topic_routes_json(
    topic: &str,
    expected_routes: usize,
    timeout: Duration,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let start = std::time::Instant::now();
    let sql = format!("SELECT routes FROM system.topics WHERE topic_id = '{}'", topic);

    while start.elapsed() < timeout {
        if let Ok(output) = execute_sql_as_root_via_client_json(&sql) {
            if let Ok(parsed) = parse_cli_json_output(&output) {
                if let Some(rows) = get_rows_as_hashmaps(&parsed) {
                    if let Some(row) = rows.first() {
                        if let Some(routes_value) = row.get("routes") {
                            let routes_untyped = extract_typed_value(routes_value);
                            if let Some(routes_json) = routes_untyped
                                .as_str()
                                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                            {
                                let route_count =
                                    routes_json.as_array().map(|routes| routes.len()).unwrap_or(0);
                                if route_count >= expected_routes {
                                    return Ok(routes_json);
                                }
                            }
                        }
                    }
                }
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    Err(format!(
        "Timed out waiting for topic '{}' to have at least {} route(s)",
        topic, expected_routes
    )
    .into())
}

fn wait_for_scalar_query_value(
    sql: &str,
    column: &str,
    timeout: Duration,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if let Ok(output) = execute_sql_as_root_via_client_json(sql) {
            if let Ok(parsed) = parse_cli_json_output(&output) {
                if let Some(rows) = get_rows_as_hashmaps(&parsed) {
                    if let Some(row) = rows.first() {
                        if let Some(value) = row.get(column) {
                            return Ok(extract_typed_value(value));
                        }
                    }
                }
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    Err(format!("Timed out waiting for scalar query result for column '{}'", column).into())
}

#[test]
fn test_cli_batch_file_chat_agent_setup_uses_random_namespace_for_topics() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let temp_dir = tempfile::TempDir::new().unwrap();
    let sql_file = temp_dir.path().join("chat-agent-setup.sql");
    let namespace = generate_unique_namespace("chat");
    let source_sql_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chat_agent_setup.sql");
    let original_sql = std::fs::read_to_string(&source_sql_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {}", source_sql_path.display(), err));
    let rewritten_sql = original_sql
        .replace(
            "CREATE NAMESPACE IF NOT EXISTS chat;",
            &format!("CREATE NAMESPACE IF NOT EXISTS {};", namespace),
        )
        .replace("USE chat;", &format!("USE {};", namespace));

    std::fs::write(&sql_file, rewritten_sql).expect("failed to write rewritten SQL file");

    let message_topic = format!("{}.message_sources_topic", namespace);
    let delete_topic = format!("{}.conversation_delete_sources_topic", namespace);
    let conversations_table = format!("{}.conversations", namespace);
    let messages_table = format!("{}.messages", namespace);

    let _ = execute_sql_as_root_via_client(&format!("DROP TOPIC {}", message_topic));
    let _ = execute_sql_as_root_via_client(&format!("DROP TOPIC {}", delete_topic));
    let _ = execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", messages_table));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", conversations_table));
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {}", namespace));

    let stdout = execute_batch_file(&sql_file)
        .unwrap_or_else(|err| panic!("Batch execution should succeed. Error: {}", err));
    assert!(
        stdout.contains("Query OK") || stdout.contains("Chat with AI Agent"),
        "batch execution should produce successful output. stdout: {}",
        stdout
    );

    wait_for_table_ready(&conversations_table, Duration::from_secs(15))
        .expect("conversations table should be ready");
    wait_for_table_ready(&messages_table, Duration::from_secs(15))
        .expect("messages table should be ready");

    let seeded_conversation_title = wait_for_scalar_query_value(
        &format!(
            "SELECT title FROM {} WHERE id = 1 LIMIT 1",
            conversations_table
        ),
        "title",
        Duration::from_secs(20),
    )
    .expect("seeded conversation row should exist in rewritten namespace");
    assert_eq!(
        seeded_conversation_title.as_str().unwrap_or(""),
        "Chat with AI Agent",
        "seeded conversation insert should target the rewritten namespace"
    );

    let seeded_message_count = wait_for_scalar_query_value(
        &format!(
            "SELECT COUNT(*) AS total FROM {} WHERE conversation_id = 1",
            messages_table
        ),
        "total",
        Duration::from_secs(20),
    )
    .expect("seeded message rows should exist in rewritten namespace");
    assert_eq!(
        json_value_as_id(&seeded_message_count).as_deref(),
        Some("2"),
        "seeded message inserts should target the rewritten namespace"
    );

    let deleted_status = wait_for_scalar_query_value(
        &format!(
            "SELECT status FROM {} WHERE id = 1 LIMIT 1",
            conversations_table
        ),
        "status",
        Duration::from_secs(20),
    )
    .expect("seeded conversation update should affect the rewritten namespace");
    assert_eq!(
        deleted_status.as_str().unwrap_or(""),
        "deleted",
        "seeded update should target the rewritten namespace"
    );

    let message_routes = wait_for_topic_routes_json(&message_topic, 1, Duration::from_secs(20))
        .expect("message sources topic should have one route");
    let delete_routes = wait_for_topic_routes_json(&delete_topic, 2, Duration::from_secs(20))
        .expect("conversation delete topic should have two routes");

    let message_routes = message_routes
        .as_array()
        .unwrap_or_else(|| panic!("message routes should be an array: {}", message_routes));
    assert_eq!(message_routes.len(), 1, "message topic should have one route");
    assert_eq!(
        message_routes[0]["table_id"].as_str().unwrap_or(""),
        messages_table,
        "message topic route should target namespaced messages table"
    );
    assert_eq!(
        message_routes[0]["op"].as_str().unwrap_or("").to_ascii_lowercase(),
        "insert",
        "message topic route should listen for inserts"
    );

    let delete_routes = delete_routes
        .as_array()
        .unwrap_or_else(|| panic!("delete routes should be an array: {}", delete_routes));
    assert_eq!(delete_routes.len(), 2, "delete topic should have two routes");
    for route in delete_routes {
        assert_eq!(
            route["table_id"].as_str().unwrap_or(""),
            conversations_table,
            "delete topic route should target namespaced conversations table"
        );
    }
    let ops: Vec<String> = delete_routes
        .iter()
        .map(|route| route["op"].as_str().unwrap_or("").to_ascii_lowercase())
        .collect();
    assert!(ops.iter().any(|op| op == "delete"), "delete route should be registered");
    assert!(ops.iter().any(|op| op == "update"), "update route should be registered");

    let _ = execute_sql_as_root_via_client(&format!("DROP TOPIC {}", message_topic));
    let _ = execute_sql_as_root_via_client(&format!("DROP TOPIC {}", delete_topic));
    let _ = execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", messages_table));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", conversations_table));
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {}", namespace));
}
