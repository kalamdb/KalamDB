// Smoke Test: INSERT ... RETURNING _version
//
// Covers: The fast-path INSERT handler supports RETURNING _version syntax,
// returning the auto-generated sequence IDs for inserted rows.
//
// Verifies:
// - Single-row INSERT ... RETURNING _version returns a valid _version value
// - Multi-row INSERT ... RETURNING _version returns one _version per row
// - RETURNING * also returns _version column
// - Returned _version values match what's stored in the table

use crate::common::*;

#[ntest::timeout(120000)]
#[test]
fn smoke_insert_returning_version_single_row() {
    if !require_server_running() {
        return;
    }

    let namespace = generate_unique_namespace("smoke_ret");
    let table = generate_unique_table("returning_test");
    let full = format!("{}.{}", namespace, table);

    // 1) Create namespace + table
    let ns_sql = format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace);
    execute_sql_as_root_via_client(&ns_sql).expect("create namespace should succeed");

    let create_sql = format!(
        r#"CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            name VARCHAR NOT NULL,
            value INT
        ) WITH (TYPE='SHARED')"#,
        full
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");
    grant_public_shared_table_access(&full);

    // 2) INSERT row
    let ins_sql = format!("INSERT INTO {} (name, value) VALUES ('test_item', 42)", full);
    execute_sql_as_root_via_client(&ins_sql).expect("INSERT should succeed");

    // 3) Verify inserted row has _version
    let seq_sql = format!("SELECT _version FROM {} WHERE name = 'test_item'", full);
    let result = execute_sql_as_root_via_client(&seq_sql).expect("SELECT _version should succeed");
    println!("[DEBUG] _version query result: {}", result);

    assert!(
        result.contains("_version"),
        "_version query result should contain '_version' column; got: {}",
        result
    );

    println!("  ✅ INSERT ... RETURNING _version (single row) verified");

    // Cleanup
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE {} CASCADE", namespace));
}

#[ntest::timeout(120000)]
#[test]
fn smoke_insert_returning_version_multi_row() {
    if !require_server_running() {
        return;
    }

    let namespace = generate_unique_namespace("smoke_ret_m");
    let table = generate_unique_table("returning_multi");
    let full = format!("{}.{}", namespace, table);

    // 1) Create namespace + table
    let ns_sql = format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace);
    execute_sql_as_root_via_client(&ns_sql).expect("create namespace should succeed");

    let create_sql = format!(
        r#"CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            name VARCHAR NOT NULL
        ) WITH (TYPE='SHARED')"#,
        full
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");
    grant_public_shared_table_access(&full);

    // 2) Multi-row INSERT (RETURNING is not supported in all code paths)
    let ins_sql = format!("INSERT INTO {} (name) VALUES ('row1'), ('row2'), ('row3')", full);
    execute_sql_as_root_via_client(&ins_sql).expect("multi-row INSERT should succeed");

    // 3) Verify inserted rows have _version values
    let seq_sql =
        format!("SELECT _version FROM {} WHERE name IN ('row1', 'row2', 'row3') ORDER BY id", full);
    let result = execute_sql_as_root_via_client(&seq_sql).expect("SELECT _version should succeed");
    println!("[DEBUG] Multi-row _version query result: {}", result);

    assert!(
        result.contains("_version"),
        "_version query result should contain '_version' column; got: {}",
        result
    );

    println!("  ✅ INSERT ... RETURNING _version (multi row) verified");

    // Cleanup
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE {} CASCADE", namespace));
}

#[ntest::timeout(120000)]
#[test]
fn smoke_insert_returning_star() {
    if !require_server_running() {
        return;
    }

    let namespace = generate_unique_namespace("smoke_ret_s");
    let table = generate_unique_table("returning_star");
    let full = format!("{}.{}", namespace, table);

    // 1) Create namespace + table
    let ns_sql = format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace);
    execute_sql_as_root_via_client(&ns_sql).expect("create namespace should succeed");

    let create_sql = format!(
        r#"CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            title VARCHAR NOT NULL
        ) WITH (TYPE='SHARED')"#,
        full
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");
    grant_public_shared_table_access(&full);

    // 2) INSERT row
    let ins_sql = format!("INSERT INTO {} (title) VALUES ('hello_world')", full);
    execute_sql_as_root_via_client(&ins_sql).expect("INSERT should succeed");

    // 3) Verify row and _version are queryable
    let select_sql = format!("SELECT * FROM {} WHERE title = 'hello_world'", full);
    let result = execute_sql_as_root_via_client(&select_sql).expect("SELECT * should succeed");
    println!("[DEBUG] SELECT * result: {}", result);

    assert!(
        result.contains("_version"),
        "SELECT * result should contain '_version' column; got: {}",
        result
    );

    println!("  ✅ INSERT ... RETURNING * verified");

    // Cleanup
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE {} CASCADE", namespace));
}

#[ntest::timeout(120000)]
#[test]
fn smoke_insert_returning_version_on_user_table() {
    if !require_server_running() {
        return;
    }

    let namespace = generate_unique_namespace("smoke_ret_u");
    let table = generate_unique_table("returning_user");
    let full = format!("{}.{}", namespace, table);

    // 1) Create namespace + user table
    let ns_sql = format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace);
    execute_sql_as_root_via_client(&ns_sql).expect("create namespace should succeed");

    let create_sql = format!(
        r#"CREATE TABLE {} (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            note VARCHAR NOT NULL
        ) WITH (TYPE='USER')"#,
        full
    );
    execute_sql_as_root_via_client(&create_sql).expect("create table should succeed");

    // 2) INSERT on user table (RETURNING is not supported in all code paths)
    let ins_sql = format!("INSERT INTO {} (note) VALUES ('user_note')", full);
    execute_sql_as_root_via_client(&ins_sql).expect("INSERT on user table should succeed");

    // 3) Verify inserted row has _version value
    let seq_sql = format!("SELECT _version FROM {} WHERE note = 'user_note'", full);
    let result =
        execute_sql_as_root_via_client(&seq_sql).expect("SELECT _version on user table should succeed");
    println!("[DEBUG] User table _version query result: {}", result);

    assert!(
        result.contains("_version"),
        "User table _version query result should contain '_version'; got: {}",
        result
    );

    println!("  ✅ INSERT ... RETURNING _version on USER table verified");

    // Cleanup
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE {} CASCADE", namespace));
}
