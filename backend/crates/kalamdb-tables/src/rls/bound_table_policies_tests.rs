use kalamdb_commons::{
    datatypes::KalamDataType,
    models::rows::Row,
    schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
    AuthorizationRelation, BoundExprShape, InvalidationStrategy, NamespaceId, PolicyCommand,
    PolicyId, PolicyProgram, PolicyTarget, PrincipalExpr, Role, TableId, TableName, TablePolicy,
    UserId,
};
use datafusion::scalar::ScalarValue;

use super::{AuthorizationSet, BoundTablePolicies};

fn table() -> TableDefinition {
    TableDefinition::new(
        NamespaceId::new("app"),
        TableName::new("documents"),
        TableType::Shared,
        vec![
            ColumnDefinition::primary_key(1, "id", 1, KalamDataType::Text),
            ColumnDefinition::simple(2, "owner_id", 2, KalamDataType::Text),
        ],
        TableOptions::shared(),
        None)
    .unwrap()
}

fn policy(name: &str, target: PolicyTarget, program: PolicyProgram) -> TablePolicy {
    let table_id = TableId::from_strings("app", "documents");
    TablePolicy::new(
        PolicyId::new(table_id.clone(), name).unwrap(),
        table_id,
        name,
        PolicyCommand::Select,
        vec![target],
        Some("owner_id = CURRENT_USER".to_string()),
        None,
        Some(program),
        None,
        1,
        1)
}

fn row(owner_id: &str) -> Row {
    Row::from_vec(vec![(
        "owner_id".to_string(),
        ScalarValue::Utf8(Some(owner_id.to_string())))])
}

#[test]
fn subjects_are_default_denied_but_admins_bypass() {
    let user = BoundTablePolicies::bind(Vec::new(), UserId::new("alice"), Role::User, PolicyCommand::Select);
    assert!(!user.authorizes_row(&table(), &row("alice")));

    let dba = BoundTablePolicies::bind(Vec::new(), UserId::new("root"), Role::Dba, PolicyCommand::Select);
    assert!(dba.authorizes_row(&table(), &row("alice")));
}

#[test]
fn row_local_policies_bind_principal_after_catalog_load() {
    let owner = policy(
        "owner_read",
        PolicyTarget::Role(Role::User),
        PolicyProgram::RowLocal {
            expr: BoundExprShape::ColumnEqualsPrincipal {
                column_id: 2,
                principal: PrincipalExpr::CurrentUser,
            },
        });
    let bound = BoundTablePolicies::bind(
        vec![owner],
        UserId::new("alice"),
        Role::User,
        PolicyCommand::Select);

    assert!(bound.authorizes_row(&table(), &row("alice")));
    assert!(!bound.authorizes_row(&table(), &row("bob")));
}

#[test]
fn policy_targets_filter_before_permissive_or() {
    let service_only = policy(
        "service_read",
        PolicyTarget::Role(Role::Service),
        PolicyProgram::RowLocal {
            expr: BoundExprShape::Literal(true),
        });
    let bound = BoundTablePolicies::bind(
        vec![service_only],
        UserId::new("alice"),
        Role::User,
        PolicyCommand::Select);

    assert!(!bound.authorizes_row(&table(), &row("alice")));
}

#[test]
fn membership_policy_authorizes_only_keys_in_bound_set() {
    let relation = AuthorizationRelation {
        protected_table: TableId::from_strings("app", "documents"),
        protected_keys: vec![1],
        relation_table: TableId::from_strings("app", "document_members"),
        relation_keys: vec![2],
        principal_column: 1,
        principal: PrincipalExpr::CurrentUser,
        static_predicates: Vec::new(),
        dependencies: vec![TableId::from_strings("app", "document_members")],
        invalidation: InvalidationStrategy::TargetedPrincipal,
    };
    let membership = policy(
        "member_read",
        PolicyTarget::Public,
        PolicyProgram::AuthorizationRelation(relation.clone()));
    let bound = BoundTablePolicies::bind(
        vec![membership.clone()],
        UserId::new("alice"),
        Role::User,
        PolicyCommand::Select);
    let authorization_set = AuthorizationSet::from_keys(
        relation,
        vec![vec![ScalarValue::Utf8(Some("doc-1".to_string()))]]);
    let sets = std::collections::HashMap::from([(
        membership.policy_id,
        std::sync::Arc::new(authorization_set))]);
    let allowed = Row::from_vec(vec![(
        "id".to_string(),
        ScalarValue::Utf8(Some("doc-1".to_string())))]);
    let denied = Row::from_vec(vec![(
        "id".to_string(),
        ScalarValue::Utf8(Some("doc-2".to_string())))]);

    assert!(bound.authorizes_row_with_sets(&table(), &allowed, &sets));
    assert!(!bound.authorizes_row_with_sets(&table(), &denied, &sets));
}
