use kalamdb_commons::{models::SchemaId, Role};
use kalamdb_dialect::classifier::{SqlStatement, SqlStatementKind};

fn classify(sql: &str, role: Role) -> Result<SqlStatement, String> {
    SqlStatement::classify_and_parse(sql, &SchemaId::new("public"), role)
        .map_err(|error| error.user_message().to_string())
}

#[test]
fn create_user_is_identity() {
    let statement =
        classify("CREATE USER alice WITH PASSWORD 'secure123' ROLE developer", Role::Dba)
            .expect("CREATE USER");
    assert!(matches!(statement.kind(), SqlStatementKind::CreateUser(_)));
}

#[test]
fn create_user_table_is_rejected() {
    let error = classify("CREATE USER TABLE items (id BIGINT PRIMARY KEY)", Role::Dba)
        .expect_err("CREATE USER TABLE");
    assert!(error.contains("CREATE USER TABLE"), "unexpected error: {error}");
}

#[test]
fn create_table_with_user_type_stays_a_table() {
    let statement =
        classify("CREATE TABLE items (id BIGINT PRIMARY KEY) WITH (TYPE = 'USER')", Role::Dba)
            .expect("CREATE TABLE");
    assert!(matches!(statement.kind(), SqlStatementKind::CreateTable(_)));
}

#[test]
fn topic_trigger_stays_a_kalam_trigger() {
    let statement = classify(
        "CREATE TRIGGER chat.process_message ON TOPIC chat.message_created EXECUTE PROCEDURE \
         chat.on_message_created(PAYLOAD)",
        Role::Dba,
    )
    .expect("topic trigger");
    assert!(matches!(statement.kind(), SqlStatementKind::CreateTrigger(_)));
}

#[test]
fn ordinary_create_trigger_is_not_captured() {
    let result = classify(
        "CREATE TRIGGER trg BEFORE INSERT ON public.items FOR EACH ROW EXECUTE FUNCTION \
         public.touch()",
        Role::Dba,
    );
    assert!(
        result.is_err()
            || result.is_ok_and(|statement| {
                !matches!(statement.kind(), SqlStatementKind::CreateTrigger(_))
            }),
        "ordinary CREATE TRIGGER must not become a topic trigger"
    );
}

#[test]
fn ordinary_create_procedure_parses_through_shared_routine_ddl() {
    for (sql, expected_lang) in [
        ("CREATE PROCEDURE api.health() LANGUAGE sql AS $$ SELECT 1 $$", "SQL"),
        (
            "CREATE OR REPLACE PROCEDURE api.health() LANGUAGE plpgsql AS $$ BEGIN END $$",
            "PLPGSQL",
        ),
    ] {
        let statement = classify(sql, Role::Dba)
            .unwrap_or_else(|err| panic!("{sql} must parse as PostgreSQL routine DDL: {err}"));
        match statement.kind() {
            SqlStatementKind::CreateProcedure(stmt) => {
                assert_eq!(stmt.language.as_deref(), Some(expected_lang));
                assert!(stmt.body.is_some());
            },
            other => panic!("expected CreateProcedure, got {other:?}"),
        }
    }
}

#[test]
fn javascript_procedure_stays_kalam() {
    let statement =
        classify("CREATE PROCEDURE api.health() LANGUAGE JAVASCRIPT AS $$ return; $$", Role::Dba)
            .expect("javascript procedure");
    assert!(matches!(statement.kind(), SqlStatementKind::CreateProcedure(_)));
}
