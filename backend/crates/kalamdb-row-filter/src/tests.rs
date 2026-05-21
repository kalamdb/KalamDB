use datafusion_common::ScalarValue;
use kalamdb_commons::models::rows::Row;

use crate::{parse_where_clause, RowFilterError};

fn test_row() -> Row {
    Row::from_vec(vec![
        ("id".to_string(), ScalarValue::Int64(Some(42))),
        ("name".to_string(), ScalarValue::Utf8(Some("Alice".to_string()))),
        ("status".to_string(), ScalarValue::Utf8(Some("active".to_string()))),
        ("priority".to_string(), ScalarValue::Int64(Some(7))),
        ("cancelled".to_string(), ScalarValue::Boolean(Some(false))),
        ("verified".to_string(), ScalarValue::Boolean(Some(true))),
        ("note".to_string(), ScalarValue::Null),
    ])
}

#[test]
fn matches_simple_equality() {
    let filter = parse_where_clause("status = 'active'").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn rejects_non_matching_equality() {
    let filter = parse_where_clause("status = 'paused'").expect("filter should parse");
    assert!(!filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_and_expression() {
    let filter =
        parse_where_clause("status = 'active' AND priority >= 5").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_or_expression() {
    let filter =
        parse_where_clause("status = 'paused' OR priority = 7").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_not_expression() {
    let filter = parse_where_clause("NOT cancelled").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_complex_nested_expression() {
    let filter = parse_where_clause(
        "(status = 'active' AND priority >= 5) OR (cancelled = true AND name = 'Bob')",
    )
    .expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_like_literal_pattern() {
    let filter = parse_where_clause("name LIKE 'Ali%'").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_ilike_literal_pattern() {
    let filter = parse_where_clause("name ILIKE 'ali%'").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_like_escape_pattern() {
    let row = Row::from_vec(vec![(
        "label".to_string(),
        ScalarValue::Utf8(Some("deploy_100".to_string())),
    )]);
    let filter =
        parse_where_clause("label LIKE 'deploy\\_%' ESCAPE '\\'").expect("filter should parse");
    assert!(filter.matches(&row).expect("filter should evaluate"));
}

#[test]
fn matches_bare_boolean_column() {
    let filter = parse_where_clause("verified").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_boolean_literal() {
    let filter = parse_where_clause("true").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_in_list() {
    let filter = parse_where_clause("status IN ('paused', 'active')").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_not_in_list() {
    let filter =
        parse_where_clause("status NOT IN ('paused', 'queued')").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_between() {
    let filter = parse_where_clause("priority BETWEEN 5 AND 10").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_is_null() {
    let filter = parse_where_clause("note IS NULL").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn treats_missing_column_as_null() {
    let filter = parse_where_clause("archived IS NULL").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));

    let equality = parse_where_clause("archived = true").expect("filter should parse");
    assert!(!equality.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_complex_publisher_route_filter() {
    let filter = parse_where_clause(
        "((status IN ('blocked', 'cancelled') AND priority BETWEEN 5 AND 10) OR event_type ILIKE 'deploy_%') AND archived IS NULL",
    )
    .expect("filter should parse");

    let status_match = Row::from_vec(vec![
        ("status".to_string(), ScalarValue::Utf8(Some("blocked".to_string()))),
        ("priority".to_string(), ScalarValue::Int32(Some(7))),
        ("event_type".to_string(), ScalarValue::Utf8(Some("noop".to_string()))),
    ]);
    let event_type_match = Row::from_vec(vec![
        ("status".to_string(), ScalarValue::Utf8(Some("active".to_string()))),
        ("priority".to_string(), ScalarValue::Int32(Some(1))),
        ("event_type".to_string(), ScalarValue::Utf8(Some("DEPLOY_START".to_string()))),
    ]);
    let archived_match_candidate = Row::from_vec(vec![
        ("status".to_string(), ScalarValue::Utf8(Some("blocked".to_string()))),
        ("priority".to_string(), ScalarValue::Int32(Some(7))),
        ("event_type".to_string(), ScalarValue::Utf8(Some("noop".to_string()))),
        ("archived".to_string(), ScalarValue::Boolean(Some(true))),
    ]);

    assert!(filter.matches(&status_match).expect("filter should evaluate"));
    assert!(filter.matches(&event_type_match).expect("filter should evaluate"));
    assert!(!filter.matches(&archived_match_candidate).expect("filter should evaluate"));
}

#[test]
fn matches_is_not_null() {
    let filter = parse_where_clause("name IS NOT NULL").expect("filter should parse");
    assert!(filter.matches(&test_row()).expect("filter should evaluate"));
}

#[test]
fn matches_is_true_and_is_false() {
    let row = Row::from_vec(vec![
        ("ready".to_string(), ScalarValue::Boolean(Some(true))),
        ("archived".to_string(), ScalarValue::Boolean(Some(false))),
    ]);

    let ready = parse_where_clause("ready IS TRUE").expect("filter should parse");
    let archived = parse_where_clause("archived IS FALSE").expect("filter should parse");

    assert!(ready.matches(&row).expect("filter should evaluate"));
    assert!(archived.matches(&row).expect("filter should evaluate"));
}

#[test]
fn explicitly_rejects_in_subquery() {
    let error = parse_where_clause("status IN (SELECT status FROM shared_table.statuses)")
        .expect_err("subquery filters need an execution context");
    assert_eq!(error, RowFilterError::UnsupportedSubquery);
}
