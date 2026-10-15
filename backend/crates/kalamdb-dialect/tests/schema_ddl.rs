use kalamdb_commons::{models::SchemaId, Role, TableType};
use kalamdb_dialect::classifier::{SqlStatement, SqlStatementKind};

fn classify(sql: &str, role: Role) -> Result<SqlStatement, String> {
    SqlStatement::classify_and_parse(sql, &SchemaId::new("public"), role)
        .map_err(|error| error.user_message().to_string())
}

#[test]
fn create_schema_classifies_as_create_schema() {
    let statement = classify("CREATE SCHEMA app", Role::Dba).expect("CREATE SCHEMA");
    match statement.kind() {
        SqlStatementKind::CreateSchema(stmt) => {
            assert_eq!(stmt.name.as_str(), "app");
            assert!(!stmt.if_not_exists);
        },
        other => panic!("expected CREATE SCHEMA, got {other:?}"),
    }
}

#[test]
fn create_schema_if_not_exists_classifies_as_create_schema() {
    let statement = classify("CREATE SCHEMA IF NOT EXISTS app", Role::Dba)
        .expect("CREATE SCHEMA IF NOT EXISTS");
    match statement.kind() {
        SqlStatementKind::CreateSchema(stmt) => {
            assert_eq!(stmt.name.as_str(), "app");
            assert!(stmt.if_not_exists);
        },
        other => panic!("expected CREATE SCHEMA, got {other:?}"),
    }
}

#[test]
fn ordinary_create_table_defaults_to_shared() {
    let statement =
        classify("CREATE TABLE items (id BIGINT PRIMARY KEY)", Role::Dba).expect("CREATE TABLE");
    match statement.kind() {
        SqlStatementKind::CreateTable(stmt) => assert_eq!(stmt.table_type, TableType::Shared),
        other => panic!("expected CREATE TABLE, got {other:?}"),
    }
}

#[test]
fn create_namespace_is_rejected() {
    for sql in ["CREATE NAMESPACE app", "CREATE NAMESPACE IF NOT EXISTS app"] {
        let error = classify(sql, Role::Dba).expect_err(sql);
        assert!(error.contains("CREATE NAMESPACE"), "{sql}: {error}");
        assert!(error.contains("CREATE SCHEMA"), "{sql}: {error}");
    }
}

#[test]
fn create_namespace_is_rejected_for_a_non_admin() {
    let error = classify("CREATE NAMESPACE app", Role::User).expect_err("CREATE NAMESPACE");
    assert!(error.contains("CREATE SCHEMA"), "{error}");
    assert!(!error.contains("Admin privileges"), "{error}");
}

#[test]
fn alter_show_and_set_schema_classify() {
    let altered = classify("ALTER SCHEMA app SET OPTIONS (region = 'us-west')", Role::Dba)
        .expect("ALTER SCHEMA");
    assert!(matches!(altered.kind(), SqlStatementKind::AlterSchema(_)));

    let shown = classify("SHOW SCHEMAS", Role::User).expect("SHOW SCHEMAS");
    assert!(matches!(shown.kind(), SqlStatementKind::ShowSchemas(_)));

    let selected = classify("SET SCHEMA app", Role::User).expect("SET SCHEMA");
    match selected.kind() {
        SqlStatementKind::UseSchema(stmt) => assert_eq!(stmt.namespace.as_str(), "app"),
        other => panic!("expected SET SCHEMA, got {other:?}"),
    }
}

#[test]
fn drop_schema_classifies_as_drop_namespace() {
    let statement = classify("DROP SCHEMA IF EXISTS app CASCADE", Role::Dba).expect("DROP SCHEMA");
    match statement.kind() {
        SqlStatementKind::DropSchema(stmt) => {
            assert_eq!(stmt.name.as_str(), "app");
            assert!(stmt.if_exists);
            assert!(stmt.cascade);
        },
        other => panic!("expected DROP SCHEMA, got {other:?}"),
    }
}

#[test]
fn create_user_remains_identity() {
    let statement =
        classify("CREATE USER alice WITH PASSWORD 'secure123' ROLE developer", Role::Dba)
            .expect("CREATE USER");
    assert!(matches!(statement.kind(), SqlStatementKind::CreateUser(_)));
}
