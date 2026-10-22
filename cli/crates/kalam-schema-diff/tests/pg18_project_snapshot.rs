use kalam_schema_diff::{diff_schema_sql_with_options, DiffOptions};
use kalamdb_sql::{compile_contract_sql, contracts::ContractTypeKind};

const FIXTURE_SQL: &str = include_str!("../../../tests/pg18_migration_fixture.sql");

#[test]
fn pg18_project_snapshot_parses_single_contract_ir_and_respects_default_schema() {
    let snapshot = compile_contract_sql(FIXTURE_SQL, "public")
        .expect("compile PG18 project snapshot into single contract IR");

    assert!(snapshot.types.contains_key("mig_app.order_state"));
    assert!(snapshot.types.contains_key("mig_app.shipping_addr"));
    assert!(snapshot.types.contains_key("mig_app.positive_qty"));
    assert!(snapshot.tables.contains_key("mig_app.orders"));

    let domain = snapshot.types.get("mig_app.positive_qty").expect("domain in snapshot");
    match &domain.kind {
        ContractTypeKind::Domain {
            base_type,
            not_null,
            checks,
            ..
        } => {
            assert_eq!(base_type.type_name.to_ascii_uppercase(), "INT");
            assert!(*not_null);
            assert_eq!(checks.len(), 1);
        },
        other => panic!("expected Domain kind, got {other:?}"),
    }

    let unqualified_sql = r#"
        CREATE TYPE local_status AS ENUM ('open', 'closed');
        CREATE DOMAIN local_code AS TEXT NOT NULL;
        CREATE TABLE local_items (
            id BIGINT PRIMARY KEY,
            status local_status NOT NULL,
            code local_code NOT NULL
        );
    "#;
    let custom_snapshot =
        compile_contract_sql(unqualified_sql, "tenant_a").expect("compile with default_schema");
    assert!(custom_snapshot.types.contains_key("tenant_a.local_status"));
    assert!(custom_snapshot.types.contains_key("tenant_a.local_code"));
    assert!(custom_snapshot.tables.contains_key("tenant_a.local_items"));

    let diff = diff_schema_sql_with_options(
        "",
        unqualified_sql,
        &DiffOptions {
            allow_destructive: false,
            default_schema:    Some("tenant_a".to_string()),
        },
    )
    .expect("diff with default_schema");
    assert!(diff
        .up
        .contains("CREATE TYPE tenant_a.local_status AS ENUM ('open', 'closed');"));
    assert!(diff.up.contains("CREATE DOMAIN tenant_a.local_code AS TEXT NOT NULL;"));
}
