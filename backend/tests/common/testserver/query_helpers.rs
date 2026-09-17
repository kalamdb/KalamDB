//! Consolidated test helpers for query execution.
//!
//! This module provides utilities for working with query responses in tests.
//! It uses the built-in helpers from `kalam_client::models::QueryResponse` where
//! possible, and adds test-specific utilities for common patterns.
//!
//! # Core Principle
//! - Use `QueryResponse` built-in methods: `rows_as_maps()`, `first_row_as_map()`, `get_i64()`,
//!   `get_string()`
//! - Add test-specific helpers here for common assertions and patterns
//! - Keep all query helpers in this single file

use kalam_client::{
    models::{QueryResponse, ResponseStatus},
    KalamCellValue,
};
use serde_json::Value as JsonValue;

/// Get a count value from a COUNT(*) query response safely.
///
/// Tries multiple column names: "count", "COUNT(*)", "total"
/// Returns the provided default if no count is found.
///
/// # Example
/// ```ignore
/// let response = server.execute_sql("SELECT COUNT(*) as total FROM users").await;
/// let count = get_count_value(&response, 0);
/// assert_eq!(count, 10);
/// ```
pub fn get_count_value(response: &QueryResponse, default: i64) -> i64 {
    response
        .first_row_as_map()
        .and_then(|row| {
            // Try multiple column names that COUNT queries might use
            row.get("count")
                .or_else(|| row.get("COUNT(*)"))
                .or_else(|| row.get("total"))
                .and_then(|v| match v.inner() {
                    JsonValue::Number(n) => n.as_i64(),
                    JsonValue::String(s) => s.parse::<i64>().ok(),
                    _ => None,
                })
        })
        .unwrap_or(default)
}

/// Assert that a query response succeeded.
///
/// # Example
/// ```ignore
/// let response = server.execute_sql("SELECT 1").await;
/// assert_query_success(&response, "SELECT 1 should succeed");
/// ```
pub fn assert_query_success(response: &QueryResponse, context: &str) {
    assert_eq!(
        response.status,
        ResponseStatus::Success,
        "{}: SQL failed: {:?}",
        context,
        response.error
    );
}

/// Assert that a query response succeeded and has at least one result.
///
/// # Example
/// ```ignore
/// let response = server.execute_sql("SELECT * FROM users").await;
/// assert_query_has_results(&response, "SELECT should return results");
/// ```
pub fn assert_query_has_results(response: &QueryResponse, context: &str) {
    assert_query_success(response, context);
    assert!(
        !response.results.is_empty(),
        "{}: Query succeeded but returned no results",
        context
    );
}

/// Assert that a query response succeeded and returned the expected row count.
///
/// # Example
/// ```ignore
/// let response = server.execute_sql("SELECT * FROM users").await;
/// assert_row_count(&response, 10, "Should have 10 users");
/// ```
pub fn assert_row_count(response: &QueryResponse, expected: usize, context: &str) {
    assert_query_success(response, context);
    let actual = response.row_count();
    assert_eq!(actual, expected, "{}: Expected {} rows, got {}", context, expected, actual);
}

/// Get a value from the first row safely, with a default.
///
/// # Example
/// ```ignore
/// let response = server.execute_sql("SELECT name FROM users LIMIT 1").await;
/// let name = get_value_or_default(&response, "name", "unknown".into());
/// ```
pub fn get_value_or_default(
    response: &QueryResponse,
    column_name: &str,
    default: KalamCellValue,
) -> KalamCellValue {
    response.get_value(column_name).unwrap_or(default)
}

/// Get an i64 value from the first row safely, with a default.
///
/// Handles both numeric and string-encoded integers.
///
/// # Example
/// ```ignore
/// let response = server.execute_sql("SELECT id FROM users LIMIT 1").await;
/// let id = get_i64_or_default(&response, "id", 0);
/// ```
pub fn get_i64_or_default(response: &QueryResponse, column_name: &str, default: i64) -> i64 {
    response.get_i64(column_name).unwrap_or(default)
}

/// Get a string value from the first row safely, with a default.
///
/// # Example
/// ```ignore
/// let response = server.execute_sql("SELECT name FROM users LIMIT 1").await;
/// let name = get_string_or_default(&response, "name", "unknown");
/// ```
pub fn get_string_or_default(response: &QueryResponse, column_name: &str, default: &str) -> String {
    response.get_string(column_name).unwrap_or_else(|| default.to_string())
}

/// Concatenate `EXPLAIN` / `EXPLAIN ANALYZE` rows into a single plan transcript.
pub fn explain_plan_text(response: &QueryResponse) -> String {
    let mut lines = Vec::new();
    let Some(result) = response.results.first() else {
        return String::new();
    };

    for row in result.rows_as_maps() {
        let plan_type = row.get("plan_type").and_then(|value| value.as_text()).unwrap_or_default();
        let plan = row.get("plan").and_then(|value| value.as_text()).unwrap_or_default();
        lines.push(format!("{plan_type} | {plan}"));
    }

    lines.join("\n")
}

/// Assert that an `EXPLAIN ANALYZE` response contains expected scan-target fragments.
pub fn assert_explain_analyze_contains(
    response: &QueryResponse,
    expected_fragments: &[&str],
    context: &str,
) {
    assert_query_success(response, context);
    let plan_text = explain_plan_text(response);
    for fragment in expected_fragments {
        assert!(
            plan_text.contains(fragment),
            "{context}: expected '{fragment}' in EXPLAIN ANALYZE output:\n{plan_text}"
        );
    }
}

/// Largest `hot_rows_scanned=` counter in an `EXPLAIN ANALYZE` transcript.
///
/// Scalar-index seeks must report this as the matching key count, not the table
/// size. `COUNT(*)` result correctness does not prove a seek happened.
pub fn hot_rows_scanned(plan_text: &str) -> Option<u64> {
    let needle = "hot_rows_scanned=";
    let mut found = None;
    let mut rest = plan_text;
    while let Some(idx) = rest.find(needle) {
        let after = &rest[idx + needle.len()..];
        let digits: String = after.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if let Ok(value) = digits.parse::<u64>() {
            found = Some(found.map_or(value, |previous: u64| previous.max(value)));
        }
        rest = after;
    }
    found
}

/// Assert SQL used a hot scalar-index seek instead of scanning the whole table.
pub fn assert_hot_index_seek(
    response: &QueryResponse,
    expected_hot_rows: u64,
    table_rows: u64,
    context: &str,
) {
    assert_query_success(response, context);
    let plan_text = explain_plan_text(response);
    let scanned = hot_rows_scanned(&plan_text).unwrap_or_else(|| {
        panic!("{context}: EXPLAIN ANALYZE missing hot_rows_scanned:\n{plan_text}")
    });
    assert!(
        expected_hot_rows < table_rows,
        "{context}: seek assertion needs a selective predicate ({expected_hot_rows} matching of \
         {table_rows} table rows)"
    );
    assert_eq!(
        scanned, expected_hot_rows,
        "{context}: expected hot_rows_scanned={expected_hot_rows} (index seek), not {scanned} \
         (full table is {table_rows}):\n{plan_text}"
    );
}

/// Assert the hot scan walked the whole table (no usable scalar seek).
pub fn assert_hot_full_scan(response: &QueryResponse, table_rows: u64, context: &str) {
    assert_query_success(response, context);
    let plan_text = explain_plan_text(response);
    let scanned = hot_rows_scanned(&plan_text).unwrap_or_else(|| {
        panic!("{context}: EXPLAIN ANALYZE missing hot_rows_scanned:\n{plan_text}")
    });
    assert_eq!(
        scanned, table_rows,
        "{context}: expected a full hot scan of {table_rows} rows, got \
         hot_rows_scanned={scanned}:\n{plan_text}"
    );
}
