use kalamdb_commons::{models::NamespaceId, Role};

use super::{
    AlterPolicyOperation, AlterPolicyStatement, CreatePolicyStatement, DropPolicyStatement,
    PolicyCommand, PolicyTarget,
};
use crate::{classify_statement, SqlStatementKind};

fn default_namespace() -> NamespaceId {
    NamespaceId::new("app")
}

#[test]
fn parses_create_policy_with_postgres_defaults() {
    let statement = CreatePolicyStatement::parse(
        "CREATE POLICY owner_read ON documents USING (owner_id = CURRENT_USER())",
        &default_namespace(),
    )
    .expect("policy should parse");

    assert_eq!(statement.policy_name, "owner_read");
    assert_eq!(statement.table_id.full_name(), "app.documents");
    assert_eq!(statement.command, PolicyCommand::All);
    assert_eq!(statement.targets, vec![PolicyTarget::Public]);
    assert_eq!(statement.using_sql.as_deref(), Some("owner_id = CURRENT_USER"));
    assert_eq!(statement.with_check_sql, None);
}

#[test]
fn parses_create_policy_roles_and_with_check() {
    let statement = CreatePolicyStatement::parse(
        "CREATE POLICY member_write ON chat.messages AS PERMISSIVE FOR UPDATE \
         TO user, service USING (owner_id = CURRENT_USER) \
         WITH CHECK (owner_id = CURRENT_USER)",
        &default_namespace(),
    )
    .expect("policy should parse");

    assert_eq!(statement.table_id.full_name(), "chat.messages");
    assert_eq!(statement.command, PolicyCommand::Update);
    assert_eq!(
        statement.targets,
        vec![PolicyTarget::Role(Role::User), PolicyTarget::Role(Role::Service)]
    );
    assert_eq!(statement.using_sql.as_deref(), Some("owner_id = CURRENT_USER"));
    assert_eq!(statement.with_check_sql.as_deref(), Some("owner_id = CURRENT_USER"));
}

#[test]
fn rejects_restrictive_policy() {
    let error = CreatePolicyStatement::parse(
        "CREATE POLICY deny_other ON documents AS RESTRICTIVE USING (owner_id = CURRENT_USER)",
        &default_namespace(),
    )
    .expect_err("restrictive policies are out of v1 scope");

    assert!(error.contains("RESTRICTIVE policies are not supported"));
}

#[test]
fn rejects_invalid_command_clauses() {
    let insert_using = CreatePolicyStatement::parse(
        "CREATE POLICY bad_insert ON documents FOR INSERT USING (true)",
        &default_namespace(),
    )
    .expect_err("INSERT may only define WITH CHECK");
    assert!(insert_using.contains("INSERT policies cannot define USING"));

    let select_check = CreatePolicyStatement::parse(
        "CREATE POLICY bad_select ON documents FOR SELECT WITH CHECK (true)",
        &default_namespace(),
    )
    .expect_err("SELECT may only define USING");
    assert!(select_check.contains("SELECT policies cannot define WITH CHECK"));
}

#[test]
fn parses_alter_policy_apply_and_rename() {
    let apply = AlterPolicyStatement::parse(
        "ALTER POLICY owner_read ON documents TO service USING (owner_id = CURRENT_USER)",
        &default_namespace(),
    )
    .expect("ALTER POLICY should parse");
    assert_eq!(apply.table_id.full_name(), "app.documents");
    assert!(matches!(
        apply.operation,
        AlterPolicyOperation::Apply {
            targets: Some(ref targets),
            using_sql: Some(ref using_sql),
            with_check_sql: None,
        } if targets == &vec![PolicyTarget::Role(Role::Service)]
            && using_sql == "owner_id = CURRENT_USER"
    ));

    let rename = AlterPolicyStatement::parse(
        "ALTER POLICY owner_read ON app.documents RENAME TO document_owner_read",
        &default_namespace(),
    )
    .expect("policy rename should parse");
    assert!(matches!(
        rename.operation,
        AlterPolicyOperation::Rename { ref new_name } if new_name == "document_owner_read"
    ));
}

#[test]
fn parses_drop_policy_if_exists() {
    let statement = DropPolicyStatement::parse(
        "DROP POLICY IF EXISTS owner_read ON documents",
        &default_namespace(),
    )
    .expect("DROP POLICY should parse");

    assert_eq!(statement.policy_name, "owner_read");
    assert_eq!(statement.table_id.full_name(), "app.documents");
    assert!(statement.if_exists);
}

#[test]
fn rejects_unknown_policy_target() {
    let error = CreatePolicyStatement::parse(
        "CREATE POLICY owner_read ON documents TO alice USING (true)",
        &default_namespace(),
    )
    .expect_err("KalamDB policy targets are role classes, not user names");

    assert!(error.contains("unsupported policy target 'alice'"));
}

#[test]
fn classifier_routes_policy_ddl_for_authorized_roles() {
    for role in [Role::Service, Role::Dba, Role::System] {
        let create = classify_statement(
            "CREATE POLICY owner_read ON documents USING (owner_id = CURRENT_USER)",
            &default_namespace(),
            role,
        )
        .expect("authorized role should classify CREATE POLICY");
        assert!(matches!(create.kind(), SqlStatementKind::CreatePolicy(_)));

        let alter = classify_statement(
            "ALTER POLICY owner_read ON documents USING (true)",
            &default_namespace(),
            role,
        )
        .expect("authorized role should classify ALTER POLICY");
        assert!(matches!(alter.kind(), SqlStatementKind::AlterPolicy(_)));

        let drop = classify_statement(
            "DROP POLICY owner_read ON documents",
            &default_namespace(),
            role,
        )
        .expect("authorized role should classify DROP POLICY");
        assert!(matches!(drop.kind(), SqlStatementKind::DropPolicy(_)));
    }
}

#[test]
fn classifier_denies_policy_ddl_for_user_role() {
    let error = classify_statement(
        "CREATE POLICY owner_read ON documents USING (true)",
        &default_namespace(),
        Role::User,
    )
    .expect_err("users cannot change table authorization policy");

    assert!(error.to_string().contains("System, DBA, or Service"));
}
