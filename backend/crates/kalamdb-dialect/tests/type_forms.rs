use kalamdb_commons::{models::SchemaId, Role};
use kalamdb_dialect::{
    classifier::{SqlStatement, SqlStatementKind, StatementClassificationError},
    ddl::{AlterDomainOperation, AlterTypeOperation, CommentOnTarget, CreateTypeBody},
};

fn classify(sql: &str) -> Result<SqlStatement, StatementClassificationError> {
    SqlStatement::classify_and_parse(sql, &SchemaId::default(), Role::System)
}

#[test]
fn test_pg18_composite_and_enum_type_forms() {
    let empty_stmt = classify("CREATE TYPE app.empty_rec AS ()")
        .expect("0-attribute composite type should parse");
    match empty_stmt.kind() {
        SqlStatementKind::CreateType(stmt) => {
            assert_eq!(stmt.namespace_id.as_str(), "app");
            assert_eq!(stmt.name, "empty_rec");
            assert_eq!(stmt.body, CreateTypeBody::Composite { fields: vec![] });
        },
        other => panic!("unexpected kind: {other:?}"),
    }

    let enum_stmt =
        classify("CREATE TYPE app.empty_enum AS ENUM ()").expect("0-label enum type should parse");
    match enum_stmt.kind() {
        SqlStatementKind::CreateType(stmt) => {
            assert_eq!(stmt.body, CreateTypeBody::Enum { labels: vec![] });
        },
        other => panic!("unexpected kind: {other:?}"),
    }

    let quoted_stmt =
        classify(r#"CREATE TYPE "App"."MyType" AS ("FieldOne" "INT", plain_field INT)"#)
            .expect("quoted composite type should parse");
    match quoted_stmt.kind() {
        SqlStatementKind::CreateType(stmt) => {
            assert_eq!(stmt.namespace_id.as_str(), "app");
            assert_eq!(stmt.name, "MyType");
            let CreateTypeBody::Composite { fields } = &stmt.body else {
                panic!("expected composite");
            };
            assert_eq!(fields[0].name, "FieldOne");
            assert!(
                fields[0].type_ref.builtin_data_type().is_none(),
                "quoted \"INT\" must resolve as a user type, not builtin INT"
            );
            assert!(
                fields[1].type_ref.builtin_data_type().is_some(),
                "unquoted INT must resolve as builtin INT"
            );
        },
        other => panic!("unexpected kind: {other:?}"),
    }
}

#[test]
fn test_pg18_nonstandard_type_forms_rejected() {
    let from_table = classify("CREATE TYPE app.customer_row FROM TABLE app.customers");
    assert!(from_table.is_err(), "FROM TABLE must be rejected");

    let inline_comment = classify("CREATE TYPE app.money AS (cents BIGINT) COMMENT 'inline'");
    assert!(inline_comment.is_err(), "inline COMMENT on CREATE TYPE must be rejected");

    let range_type = classify("CREATE TYPE app.float_range AS RANGE (SUBTYPE = float8)");
    assert!(
        range_type
            .unwrap_err()
            .to_string()
            .contains("CREATE TYPE ... AS RANGE is not supported"),
        "AS RANGE must fail with explicit diagnostic"
    );

    let shell_type = classify("CREATE TYPE app.shell_only");
    assert!(
        shell_type.unwrap_err().to_string().to_ascii_lowercase().contains("shell type"),
        "shell CREATE TYPE must fail with explicit diagnostic"
    );

    let base_type = classify("CREATE TYPE app.base_t (INPUT = in_fn, OUTPUT = out_fn)");
    assert!(
        base_type.unwrap_err().to_string().to_ascii_lowercase().contains("base type"),
        "base CREATE TYPE must fail with explicit diagnostic"
    );
}

#[test]
fn test_pg18_domain_ddl_forms() {
    let create_dom = classify(
        "CREATE DOMAIN app.pos_int AS INTEGER DEFAULT 1 NOT NULL CONSTRAINT pos_chk CHECK (VALUE \
         > 0)",
    )
    .expect("CREATE DOMAIN should classify");
    match create_dom.kind() {
        SqlStatementKind::CreateDomain(stmt) => {
            assert_eq!(stmt.namespace_id.as_str(), "app");
            assert_eq!(stmt.name, "pos_int");
            assert_eq!(stmt.default_expr.as_deref(), Some("1"));
            assert!(stmt.not_null);
            assert_eq!(stmt.checks.len(), 1);
            assert_eq!(stmt.checks[0].name.as_deref(), Some("pos_chk"));
        },
        other => panic!("unexpected kind: {other:?}"),
    }

    let alter_dom =
        classify("ALTER DOMAIN app.pos_int ADD CONSTRAINT max_chk CHECK (VALUE < 1000) NOT VALID")
            .expect("ALTER DOMAIN ADD CONSTRAINT NOT VALID should classify");
    match alter_dom.kind() {
        SqlStatementKind::AlterDomain(stmt) => match &stmt.operation {
            AlterDomainOperation::AddCheck { clause } => {
                assert_eq!(clause.name.as_deref(), Some("max_chk"));
                assert!(!clause.validated);
            },
            other => panic!("unexpected op: {other:?}"),
        },
        other => panic!("unexpected kind: {other:?}"),
    }

    let validate_dom = classify("ALTER DOMAIN app.pos_int VALIDATE CONSTRAINT max_chk")
        .expect("ALTER DOMAIN VALIDATE CONSTRAINT should classify");
    assert!(matches!(validate_dom.kind(), SqlStatementKind::AlterDomain(_)));

    let drop_dom = classify("DROP DOMAIN IF EXISTS app.pos_int, app.other_dom CASCADE")
        .expect("DROP DOMAIN should classify");
    match drop_dom.kind() {
        SqlStatementKind::DropDomain(stmt) => {
            assert!(stmt.if_exists);
            assert!(stmt.cascade);
            assert_eq!(stmt.targets.len(), 2);
        },
        other => panic!("unexpected kind: {other:?}"),
    }

    let comment_dom = classify("COMMENT ON DOMAIN app.pos_int IS 'positive integer domain'")
        .expect("COMMENT ON DOMAIN should classify");
    match comment_dom.kind() {
        SqlStatementKind::CommentOn(stmt) => {
            assert!(matches!(stmt.target, CommentOnTarget::Type { .. }));
        },
        other => panic!("unexpected kind: {other:?}"),
    }
}

#[test]
fn test_pg18_alter_and_drop_type_forms() {
    let rename_val = classify("ALTER TYPE app.mood RENAME VALUE 'sad' TO 'unhappy'")
        .expect("ALTER TYPE RENAME VALUE should classify");
    match rename_val.kind() {
        SqlStatementKind::AlterType(stmt) => {
            assert_eq!(
                stmt.operation,
                AlterTypeOperation::RenameValue {
                    from: "sad".to_string(),
                    to: "unhappy".to_string(),
                }
            );
        },
        other => panic!("unexpected kind: {other:?}"),
    }

    let drop_multi = classify("DROP TYPE IF EXISTS app.t1, app.t2 RESTRICT")
        .expect("multi-target DROP TYPE should classify");
    match drop_multi.kind() {
        SqlStatementKind::DropType(stmt) => {
            assert!(stmt.if_exists);
            assert!(!stmt.cascade);
            assert_eq!(stmt.targets.len(), 2);
        },
        other => panic!("unexpected kind: {other:?}"),
    }
}
