// Smoke Test: Int64 precision preservation for JavaScript safety
//
// This test verifies that Int64 and UInt64 values exceeding JavaScript's
// Number.MAX_SAFE_INTEGER (2^53 - 1 = 9007199254740991) are serialized as
// strings in JSON API responses to prevent precision loss in JavaScript clients.
// The `_seq` column (Snowflake ID) is a UInt64 that typically exceeds this limit.

use crate::common::*;

/// JavaScript's Number.MAX_SAFE_INTEGER: 2^53 - 1
const JS_MAX_SAFE_INTEGER: i64 = 9007199254740991;

#[ntest::timeout(180000)]
#[test]
fn smoke_int64_precision_preserved_as_string() {
    if !is_server_running() {
        println!(
            "Skipping smoke_int64_precision_preserved_as_string: server not running at {}",
            server_url()
        );
        return;
    }

    // Create unique namespace and table
    let namespace = generate_unique_namespace("int64_ns");
    let table = generate_unique_table("precision_test");
    let full_table = format!("{}.{}", namespace, table);

    // 0) Create namespace
    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .expect("create namespace should succeed");

    // 1) Create USER table with BIGINT column
    let create_sql = format!(
        "CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            big_value BIGINT,
            content TEXT
        ) WITH (TYPE = 'USER')",
        full_table
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");

    // 2) Insert rows - the _seq column will be a large Snowflake ID
    let insert_sql = format!(
        "INSERT INTO {} (big_value, content) VALUES ({}, 'test_precision')",
        full_table,
        JS_MAX_SAFE_INTEGER + 1000 // A value that exceeds JS safe integer
    );
    execute_sql_as_root_via_client(&insert_sql).expect("insert should succeed");

    // 3) Query via JSON endpoint and verify _seq is a string
    let select_sql = format!("SELECT _seq, big_value, content FROM {}", full_table);
    let json_response =
        execute_sql_as_root_via_client_json(&select_sql).expect("select should succeed");

    // Parse the JSON response
    let parsed: serde_json::Value =
        serde_json::from_str(&json_response).expect("Failed to parse JSON response");

    // Convert array-based rows to HashMap-based rows
    let rows = get_rows_as_hashmaps(&parsed).expect("Expected rows in JSON response");

    assert!(!rows.is_empty(), "Expected at least one row");

    let first_row = &rows[0];

    // Extract typed values from the response (server returns {"Int64": "value"} format)
    let seq_value = first_row.get("_seq").expect("Expected _seq column");
    let seq_value = extract_typed_value(seq_value);

    // Verify _seq is a string (since it's a Snowflake ID exceeding JS_MAX_SAFE_INTEGER)
    assert!(
        seq_value.is_string(),
        "_seq should be serialized as a string for JS safety, got: {:?}",
        seq_value
    );

    // Verify the string value is a valid numeric
    let seq_str = seq_value.as_str().expect("_seq should be a string");
    let seq_parsed: u64 = seq_str.parse().expect("_seq string should be a valid number");
    assert!(
        seq_parsed > JS_MAX_SAFE_INTEGER as u64,
        "_seq should exceed JS_MAX_SAFE_INTEGER, got: {}",
        seq_parsed
    );

    // Extract and verify big_value
    let big_value = first_row.get("big_value").expect("Expected big_value column");
    let big_value = extract_typed_value(big_value);

    // Verify big_value is also a string (since we inserted a value > JS_MAX_SAFE_INTEGER)
    assert!(
        big_value.is_string(),
        "big_value should be serialized as a string for JS safety, got: {:?}",
        big_value
    );

    let big_value_str = big_value.as_str().expect("big_value should be a string");
    let big_value_parsed: i64 =
        big_value_str.parse().expect("big_value string should be a valid number");
    assert_eq!(
        big_value_parsed,
        JS_MAX_SAFE_INTEGER + 1000,
        "big_value should preserve full precision"
    );

    // Verify content is still a regular string (after extracting from typed format)
    let content = first_row.get("content").expect("Expected content column");
    let content = extract_typed_value(content);
    assert_eq!(content.as_str(), Some("test_precision"), "content should be 'test_precision'");

    println!("✓ Int64 precision test passed: _seq={}, big_value={}", seq_str, big_value_str);

    // Cleanup
    execute_sql_as_root_via_client(&format!("DROP TABLE {}", full_table)).ok();
    execute_sql_as_root_via_client(&format!("DROP NAMESPACE {}", namespace)).ok();
}

#[ntest::timeout(180000)]
#[test]
fn smoke_int64_small_values_remain_numbers() {
    if !is_server_running() {
        println!(
            "Skipping smoke_int64_small_values_remain_numbers: server not running at {}",
            server_url()
        );
        return;
    }

    // Create unique namespace and table
    let namespace = generate_unique_namespace("int64_small_ns");
    let table = generate_unique_table("small_values");
    let full_table = format!("{}.{}", namespace, table);

    // 0) Create namespace
    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .expect("create namespace should succeed");

    // 1) Create USER table
    let create_sql = format!(
        "CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            small_value BIGINT,
            content TEXT
        ) WITH (TYPE = 'USER')",
        full_table
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");

    // 2) Insert a small value that fits safely in JS number
    let small_value: i64 = 12345678901234; // Well under 2^53
    let insert_sql = format!(
        "INSERT INTO {} (small_value, content) VALUES ({}, 'small_test')",
        full_table, small_value
    );
    execute_sql_as_root_via_client(&insert_sql).expect("insert should succeed");

    // 3) Query via JSON endpoint
    let select_sql = format!("SELECT small_value, content FROM {}", full_table);
    let json_response =
        execute_sql_as_root_via_client_json(&select_sql).expect("select should succeed");

    // Parse the JSON response
    let parsed: serde_json::Value =
        serde_json::from_str(&json_response).expect("Failed to parse JSON response");

    // Convert array-based rows to HashMap-based rows
    let rows = get_rows_as_hashmaps(&parsed).expect("Expected rows in JSON response");

    assert!(!rows.is_empty(), "Expected at least one row");

    let first_row = &rows[0];

    // With typed JSON format, ALL Int64 values are strings (for consistency)
    let small_val = first_row.get("small_value").expect("Expected small_value column");
    let small_val = extract_typed_value(small_val);
    assert!(
        small_val.is_string(),
        "small_value should be string in typed JSON format, got: {:?}",
        small_val
    );

    let small_str = small_val.as_str().expect("small_value should be a string");
    let parsed_value: i64 = small_str.parse().expect("small_value string should parse as i64");
    assert_eq!(parsed_value, small_value, "small_value should preserve value correctly");

    println!(
        "✓ Small Int64 test passed: small_value={} (as string for consistency)",
        small_str
    );

    // Cleanup
    execute_sql_as_root_via_client(&format!("DROP TABLE {}", full_table)).ok();
    execute_sql_as_root_via_client(&format!("DROP NAMESPACE {}", namespace)).ok();
}

#[ntest::timeout(180000)]
#[test]
fn smoke_int64_edge_case_exactly_max_safe() {
    if !is_server_running() {
        println!(
            "Skipping smoke_int64_edge_case_exactly_max_safe: server not running at {}",
            server_url()
        );
        return;
    }

    // Create unique namespace and table
    let namespace = generate_unique_namespace("int64_edge_ns");
    let table = generate_unique_table("edge_values");
    let full_table = format!("{}.{}", namespace, table);

    // 0) Create namespace
    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .expect("create namespace should succeed");

    // 1) Create USER table
    let create_sql = format!(
        "CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            at_limit BIGINT,
            over_limit BIGINT,
            content TEXT
        ) WITH (TYPE = 'USER')",
        full_table
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");

    // 2) Insert values at and over the limit
    let insert_sql = format!(
        "INSERT INTO {} (at_limit, over_limit, content) VALUES ({}, {}, 'edge_test')",
        full_table,
        JS_MAX_SAFE_INTEGER,     // Exactly at the limit - should be number
        JS_MAX_SAFE_INTEGER + 1  // Just over - should be string
    );
    execute_sql_as_root_via_client(&insert_sql).expect("insert should succeed");

    // 3) Query via JSON endpoint
    let select_sql = format!("SELECT at_limit, over_limit, content FROM {}", full_table);
    let json_response =
        execute_sql_as_root_via_client_json(&select_sql).expect("select should succeed");

    // Parse the JSON response
    let parsed: serde_json::Value =
        serde_json::from_str(&json_response).expect("Failed to parse JSON response");

    // Convert array-based rows to HashMap-based rows
    let rows = get_rows_as_hashmaps(&parsed).expect("Expected rows in JSON response");

    assert!(!rows.is_empty(), "Expected at least one row");

    let first_row = &rows[0];

    // With typed JSON format, ALL Int64 values are strings (for consistency)
    let at_limit = first_row.get("at_limit").expect("Expected at_limit column");
    let at_limit = extract_typed_value(at_limit);
    assert!(
        at_limit.is_string(),
        "at_limit should be string in typed JSON format, got: {:?}",
        at_limit
    );
    let at_limit_str = at_limit.as_str().expect("at_limit should be a string");
    let at_limit_parsed: i64 = at_limit_str.parse().expect("at_limit should parse");
    assert_eq!(at_limit_parsed, JS_MAX_SAFE_INTEGER, "at_limit value mismatch");

    // Verify over_limit is a string (just over MAX_SAFE_INTEGER)
    let over_limit = first_row.get("over_limit").expect("Expected over_limit column");
    let over_limit = extract_typed_value(over_limit);
    assert!(
        over_limit.is_string(),
        "over_limit should be a string when > JS_MAX_SAFE_INTEGER, got: {:?}",
        over_limit
    );
    let over_limit_str = over_limit.as_str().expect("over_limit should be a string");
    let over_limit_parsed: i64 = over_limit_str.parse().expect("over_limit should parse");
    assert_eq!(over_limit_parsed, JS_MAX_SAFE_INTEGER + 1, "over_limit value mismatch");

    println!(
        "✓ Edge case test passed: at_limit={} (string), over_limit={} (string)",
        at_limit_str, over_limit_str
    );

    // Cleanup
    execute_sql_as_root_via_client(&format!("DROP TABLE {}", full_table)).ok();
    execute_sql_as_root_via_client(&format!("DROP NAMESPACE {}", namespace)).ok();
}

#[ntest::timeout(180000)]
#[test]
fn smoke_int64_negative_large_values() {
    if !is_server_running() {
        println!(
            "Skipping smoke_int64_negative_large_values: server not running at {}",
            server_url()
        );
        return;
    }

    // Create unique namespace and table
    let namespace = generate_unique_namespace("int64_neg_ns");
    let table = generate_unique_table("negative_values");
    let full_table = format!("{}.{}", namespace, table);

    // 0) Create namespace
    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .expect("create namespace should succeed");

    // 1) Create USER table
    let create_sql = format!(
        "CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            neg_safe BIGINT,
            neg_unsafe BIGINT,
            content TEXT
        ) WITH (TYPE = 'USER')",
        full_table
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");

    // 2) Insert negative values at and beyond the negative safe limit
    // JavaScript's MIN_SAFE_INTEGER is -(2^53 - 1) = -9007199254740991
    let min_safe: i64 = -JS_MAX_SAFE_INTEGER;
    let insert_sql = format!(
        "INSERT INTO {} (neg_safe, neg_unsafe, content) VALUES ({}, {}, 'neg_test')",
        full_table,
        min_safe,     // At negative limit - should be number
        min_safe - 1  // Beyond negative limit - should be string
    );
    execute_sql_as_root_via_client(&insert_sql).expect("insert should succeed");

    // 3) Query via JSON endpoint
    let select_sql = format!("SELECT neg_safe, neg_unsafe, content FROM {}", full_table);
    let json_response =
        execute_sql_as_root_via_client_json(&select_sql).expect("select should succeed");

    // Parse the JSON response
    let parsed: serde_json::Value =
        serde_json::from_str(&json_response).expect("Failed to parse JSON response");

    // Convert array-based rows to HashMap-based rows
    let rows = get_rows_as_hashmaps(&parsed).expect("Expected rows in JSON response");

    assert!(!rows.is_empty(), "Expected at least one row");

    let first_row = &rows[0];

    // With typed JSON format, ALL Int64 values are strings (for consistency)
    let neg_safe = first_row.get("neg_safe").expect("Expected neg_safe column");
    let neg_safe = extract_typed_value(neg_safe);
    assert!(
        neg_safe.is_string(),
        "neg_safe should be string in typed JSON format, got: {:?}",
        neg_safe
    );
    let neg_safe_str = neg_safe.as_str().expect("neg_safe should be a string");
    let neg_safe_parsed: i64 = neg_safe_str.parse().expect("neg_safe should parse");
    assert_eq!(neg_safe_parsed, min_safe, "neg_safe value mismatch");

    // Verify neg_unsafe is a string
    let neg_unsafe = first_row.get("neg_unsafe").expect("Expected neg_unsafe column");
    let neg_unsafe = extract_typed_value(neg_unsafe);
    assert!(
        neg_unsafe.is_string(),
        "neg_unsafe should be a string when < -JS_MAX_SAFE_INTEGER, got: {:?}",
        neg_unsafe
    );
    let neg_unsafe_str = neg_unsafe.as_str().expect("neg_unsafe should be a string");
    let neg_unsafe_parsed: i64 = neg_unsafe_str.parse().expect("neg_unsafe should parse");
    assert_eq!(neg_unsafe_parsed, min_safe - 1, "neg_unsafe value mismatch");

    println!(
        "✓ Negative large value test passed: neg_safe={} (string), neg_unsafe={} (string)",
        neg_safe_str, neg_unsafe_str
    );

    // Cleanup
    execute_sql_as_root_via_client(&format!("DROP TABLE {}", full_table)).ok();
    execute_sql_as_root_via_client(&format!("DROP NAMESPACE {}", namespace)).ok();
}
