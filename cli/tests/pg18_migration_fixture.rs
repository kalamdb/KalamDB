use std::path::Path;

use kalam_schema_diff::diff_schema_sql;

#[test]
fn pg18_migration_fixture_has_expected_schema_types_and_data() {
    let fixture_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pg18_migration_fixture.sql");
    let sql = std::fs::read_to_string(&fixture_path).expect("read pg18_migration_fixture.sql");

    assert!(sql.contains("CREATE SCHEMA IF NOT EXISTS mig_app;"));
    assert!(sql.contains("CREATE TYPE mig_app.order_state AS ENUM"));
    assert!(sql.contains("CREATE TYPE mig_app.shipping_addr AS"));
    assert!(sql.contains("CREATE DOMAIN mig_app.positive_qty"));
    assert!(sql.contains("CREATE TABLE mig_app.orders"));
    assert!(sql.contains("CREATE INDEX idx_mig_orders_customer"));
    assert!(sql.contains("INSERT INTO mig_app.orders"));

    // Extract DDL portion (before INSERT) and verify roundtrip no-op diff
    let ddl_only: String = sql
        .lines()
        .take_while(|line| !line.trim_start().starts_with("INSERT INTO"))
        .collect::<Vec<_>>()
        .join("\n");

    let noop = diff_schema_sql(&ddl_only, &ddl_only).expect("diff fixture ddl against itself");
    assert!(noop.up.contains("No schema changes"), "expected no-op diff, got:\n{}", noop.up);
}
