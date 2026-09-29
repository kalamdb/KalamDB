//! Cluster function tests.
//!
//! JavaScript procedures are defined through `CREATE PROCEDURE` and invoked
//! with `CALL`. In cluster mode a follower accepts both statements and
//! forwards them to the leader, so a client connected to any node can create
//! and call the same procedure. These tests pin each request to a specific
//! node (`execute_on_node_*_raw`) so a follower is not skipped in favor of
//! the leader by the test client.

use std::{
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::{cluster_common::*, common::*};

fn leader_and_followers() -> (String, Vec<String>) {
    let urls = cluster_urls();
    let leader = leader_url().expect("cluster leader URL");
    let followers: Vec<String> = urls.into_iter().filter(|url| url != &leader).collect();
    assert!(!followers.is_empty(), "function cluster tests need at least one follower");
    (leader, followers)
}

fn setup_namespace(label: &str) -> String {
    let namespace = generate_unique_namespace(label);
    let (leader, _) = leader_and_followers();
    let _ = execute_on_node(&leader, &format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
    execute_on_node(&leader, &format!("CREATE NAMESPACE {namespace}"))
        .unwrap_or_else(|err| panic!("create namespace {namespace}: {err}"));
    namespace
}

fn create_js_procedure(node: &str, namespace: &str, name: &str, params: &str, body: &str) {
    let mut sql = format!(
        "CREATE OR REPLACE PROCEDURE {namespace}.{name}({params}) LANGUAGE JAVASCRIPT AS $$"
    );
    sql.push_str(body);
    sql.push_str("$$");
    execute_on_node_raw(node, &sql)
        .unwrap_or_else(|err| panic!("create {namespace}.{name} on {node}: {err}"));
}

fn call_cell(node: &str, sql: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match execute_on_node_response_raw(node, sql) {
            Ok(response) => {
                let result =
                    response.results.first().unwrap_or_else(|| panic!("missing result for {sql}"));
                let rows =
                    result.rows.as_ref().unwrap_or_else(|| panic!("CALL returned no rows: {sql}"));
                assert_eq!(rows.len(), 1, "CALL should return one row: {sql}");
                let cell = rows[0].first().unwrap_or_else(|| panic!("CALL row is empty: {sql}"));
                return extract_typed_value(cell);
            },
            Err(err)
                if err.to_ascii_lowercase().contains("not implemented")
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(150));
            },
            Err(err) => panic!("CALL on {node} failed: {sql}: {err}"),
        }
    }
}

fn cell_text(value: &Value) -> String {
    value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())
}

fn cell_i64(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_else(|| panic!("expected an integer cell, got {value}"))
}

fn assert_procedure_on_leader(leader: &str, procedure_id: &str) {
    let sql =
        format!("SELECT procedure_id FROM system.procedures WHERE procedure_id = '{procedure_id}'");
    let response = execute_on_node_response(leader, &sql)
        .unwrap_or_else(|err| panic!("catalog lookup for {procedure_id}: {err}"));
    let rows = response
        .results
        .first()
        .and_then(|result| result.rows.as_ref())
        .map(|rows| rows.len())
        .unwrap_or(0);
    assert_eq!(rows, 1, "leader catalog should contain {procedure_id}");
}

/// A procedure created on the leader returns its argument when called there.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_call_from_leader() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, _) = leader_and_followers();
    let namespace = setup_namespace("fn_leader");
    create_js_procedure(&leader, &namespace, "echo", "msg TEXT", "return input.msg;");
    assert_procedure_on_leader(&leader, &format!("{namespace}.echo"));

    let value = call_cell(&leader, &format!("CALL {namespace}.echo('from-leader')"));
    assert_eq!(cell_text(&value), "from-leader");
}

/// Each follower accepts CALL and returns the same procedure result.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_call_from_followers() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, followers) = leader_and_followers();
    let namespace = setup_namespace("fn_followers");
    create_js_procedure(&leader, &namespace, "echo", "msg TEXT", "return input.msg;");

    for (index, follower) in followers.iter().enumerate() {
        let marker = format!("from-follower-{index}");
        let value = call_cell(follower, &format!("CALL {namespace}.echo('{marker}')"));
        assert_eq!(cell_text(&value), marker, "follower {follower} returned the wrong value");
    }
}

/// A procedure created on a follower is callable from the leader and the
/// other followers, including after the follower replaces the body.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_defined_on_follower() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, followers) = leader_and_followers();
    let author = &followers[0];
    let namespace = setup_namespace("fn_defined");
    create_js_procedure(author, &namespace, "add", "a INT, b INT", "return input.a + input.b;");
    assert_procedure_on_leader(&leader, &format!("{namespace}.add"));

    let leader_value = call_cell(&leader, &format!("CALL {namespace}.add(20, 22)"));
    assert_eq!(cell_i64(&leader_value), 42);

    for follower in followers.iter().skip(1) {
        let value = call_cell(follower, &format!("CALL {namespace}.add(20, 22)"));
        assert_eq!(cell_i64(&value), 42, "follower {follower}");
    }

    create_js_procedure(author, &namespace, "add", "a INT, b INT", "return input.a + input.b + 1;");
    let replaced = call_cell(&leader, &format!("CALL {namespace}.add(20, 22)"));
    assert_eq!(cell_i64(&replaced), 43, "replaced body should be visible from the leader");
}

/// A follower can invoke a procedure that calls another procedure, and a
/// thrown error is returned to the follower client.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_nested_call_from_follower() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, followers) = leader_and_followers();
    let namespace = setup_namespace("fn_nested");
    create_js_procedure(&leader, &namespace, "inc", "x INT", "return input.x + 1;");
    create_js_procedure(
        &leader,
        &namespace,
        "plus_one",
        "x INT",
        &format!("return ctx.functions.call('{namespace}.inc', [input]);"),
    );
    create_js_procedure(&leader, &namespace, "boom", "", "throw new Error('boom');");

    let caller = &followers[0];
    let nested = call_cell(caller, &format!("CALL {namespace}.plus_one(41)"));
    assert_eq!(cell_i64(&nested), 42, "nested CALL from {caller}");

    let error_node = followers.get(1).unwrap_or(caller);
    let error = execute_on_node_response_raw(error_node, &format!("CALL {namespace}.boom()"))
        .expect_err("boom should fail");
    let error_lower = error.to_ascii_lowercase();
    assert!(
        error_lower.contains("boom"),
        "follower {error_node} should surface the procedure error: {error}"
    );
}

/// A procedure called on a follower inserts a row that every node can read.
#[ntest::timeout(180_000)]
#[test]
fn cluster_test_function_write_from_follower_replicated() {
    if !require_cluster_running() {
        return;
    }
    let _function_pool = lock_function_pool();

    let (leader, followers) = leader_and_followers();
    let namespace = setup_namespace("fn_write");
    let table = format!("{namespace}.notes");
    execute_on_node(&leader, &format!("CREATE TABLE {table} (id INT PRIMARY KEY, body TEXT)"))
        .unwrap_or_else(|err| panic!("create {table}: {err}"));
    assert!(
        wait_for_table_on_all_nodes(&namespace, "notes", 15_000),
        "{table} did not appear on every node"
    );

    create_js_procedure(
        &leader,
        &namespace,
        "note",
        "id INT, body TEXT",
        &format!(
            "return ctx.db.execute(\"INSERT INTO {table} (id, body) VALUES (\" + input.id + \", \
             '\" + input.body + \"')\").then(function () {{ return input.id; }});"
        ),
    );

    let writer = &followers[0];
    let written = call_cell(writer, &format!("CALL {namespace}.note(7, 'from-follower')"));
    assert_eq!(cell_i64(&written), 7);

    let query = format!("SELECT body FROM {table} WHERE id = 7");
    let deadline = Instant::now() + Duration::from_secs(15);
    let urls = cluster_urls();
    for url in &urls {
        let mut saw_row = false;
        while Instant::now() < deadline {
            if let Ok(response) = execute_on_node_response(url, &query) {
                if let Some(rows) = response.results.first().and_then(|result| result.rows.as_ref())
                {
                    if let Some(row) = rows.first() {
                        if let Some(cell) = row.first() {
                            if cell_text(&extract_typed_value(cell)) == "from-follower" {
                                saw_row = true;
                                break;
                            }
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(200));
        }
        assert!(saw_row, "{url} never saw the row written by CALL on {writer}");
    }
}
