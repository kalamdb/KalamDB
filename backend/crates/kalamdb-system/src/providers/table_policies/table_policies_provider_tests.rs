use std::sync::Arc;

use kalamdb_commons::{
    BoundExprShape, PolicyCommand, PolicyId, PolicyProgram, PolicyTarget, PrincipalExpr, Role,
    SystemTable, TableId, TablePolicy,
};
use kalamdb_store::test_utils::InMemoryBackend;

use super::TablePoliciesTableProvider;

fn row_local_policy(table_id: TableId, name: &str, dependency_generation: u64) -> TablePolicy {
    TablePolicy::new(
        PolicyId::new(table_id.clone(), name).expect("valid policy id"),
        table_id,
        name,
        PolicyCommand::Select,
        vec![PolicyTarget::Role(Role::User)],
        Some("owner_id = CURRENT_USER".to_string()),
        None,
        Some(PolicyProgram::RowLocal {
            expr: BoundExprShape::ColumnEqualsPrincipal {
                column_id: 2,
                principal: PrincipalExpr::CurrentUser,
            },
        }),
        None,
        dependency_generation,
        1)
}

#[tokio::test]
async fn persists_policy_definitions_and_lists_them_by_table() {
    let provider = TablePoliciesTableProvider::new(Arc::new(InMemoryBackend::new()));
    let documents = TableId::from_strings("app", "documents");
    let policy = row_local_policy(documents.clone(), "owner_read", 1);

    provider.create_policy(policy.clone()).await.expect("create policy");

    assert_eq!(provider.get_policy(&policy.policy_id).await.unwrap(), Some(policy.clone()));
    assert_eq!(provider.list_for_table(&documents).unwrap(), vec![policy]);
    assert_eq!(SystemTable::TablePolicies.table_name(), "table_policies");
}

#[tokio::test]
async fn create_is_unique_and_mutations_advance_table_generation() {
    let provider = TablePoliciesTableProvider::new(Arc::new(InMemoryBackend::new()));
    let documents = TableId::from_strings("app", "documents");
    let first = row_local_policy(documents.clone(), "owner_read", 0);

    let first = provider.create_policy(first).await.expect("first create");
    assert_eq!(first.policy_generation, 1);
    assert!(provider.create_policy(first.clone()).await.is_err());

    let mut altered = first.clone();
    altered.targets = vec![PolicyTarget::Public];
    let altered = provider.replace_policy(altered).await.expect("alter policy");
    assert_eq!(altered.policy_generation, 2);

    provider.delete_policy(&altered.policy_id, false).await.expect("drop policy");
    assert_eq!(provider.policy_generation(&documents).unwrap(), 3);
}

#[tokio::test]
async fn compiled_policy_cache_is_user_independent_and_generation_scoped() {
    let provider = TablePoliciesTableProvider::new(Arc::new(InMemoryBackend::new()));
    let documents = TableId::from_strings("app", "documents_cached");
    let first = provider
        .create_policy(row_local_policy(documents.clone(), "owner_read", 0))
        .await
        .unwrap();

    let cached = provider.compiled_for_table(&documents, 1).unwrap();
    let reused = provider.compiled_for_table(&documents, 1).unwrap();
    assert!(Arc::ptr_eq(&cached, &reused));
    assert_eq!(cached.policy_generation, 1);

    let mut altered = first;
    altered.targets = vec![PolicyTarget::Public];
    provider.replace_policy(altered).await.unwrap();
    let rebound = provider.compiled_for_table(&documents, 1).unwrap();
    assert!(!Arc::ptr_eq(&cached, &rebound));
    assert_eq!(rebound.policy_generation, 2);
}

#[tokio::test]
async fn drop_table_cleanup_removes_policies_and_reverse_dependencies() {
    let provider = TablePoliciesTableProvider::new(Arc::new(InMemoryBackend::new()));
    let messages = TableId::from_strings("chat", "messages");
    let members = TableId::from_strings("chat", "members");
    let mut policy = row_local_policy(messages.clone(), "member_read", 0);
    policy.using_program = Some(PolicyProgram::AuthorizationRelation(
        kalamdb_commons::AuthorizationRelation {
            protected_table: messages.clone(),
            protected_keys: vec![2],
            relation_table: members.clone(),
            relation_keys: vec![2],
            principal_column: 1,
            principal: PrincipalExpr::CurrentUser,
            static_predicates: Vec::new(),
            dependencies: vec![members.clone()],
            invalidation: kalamdb_commons::InvalidationStrategy::TargetedPrincipal,
        }));
    let policy = provider.create_policy(policy).await.expect("create policy");

    assert_eq!(provider.dependent_policies(&members).unwrap(), vec![policy.policy_id.clone()]);

    provider.delete_for_table(&messages).await.expect("drop protected table policies");
    assert!(provider.dependent_policies(&members).unwrap().is_empty());
}

#[tokio::test]
async fn rename_changes_identity_and_advances_generation() {
    let provider = TablePoliciesTableProvider::new(Arc::new(InMemoryBackend::new()));
    let documents = TableId::from_strings("app", "documents");
    let policy = provider
        .create_policy(row_local_policy(documents.clone(), "owner_read", 0))
        .await
        .expect("create policy");

    let renamed = provider
        .rename_policy(&policy.policy_id, "document_owner_read")
        .await
        .expect("rename policy");

    assert_eq!(renamed.policy_generation, 2);
    assert_eq!(renamed.policy_name, "document_owner_read");
    assert!(provider.get_policy(&policy.policy_id).await.unwrap().is_none());
    assert_eq!(provider.get_policy(&renamed.policy_id).await.unwrap(), Some(renamed));
}

#[tokio::test]
async fn rejects_policy_dependency_cycles() {
    let provider = TablePoliciesTableProvider::new(Arc::new(InMemoryBackend::new()));
    let messages = TableId::from_strings("chat", "messages");
    let members = TableId::from_strings("chat", "members");

    let mut messages_policy = row_local_policy(messages.clone(), "via_members", 0);
    messages_policy.using_program = Some(membership_program(&messages, &members));
    provider.create_policy(messages_policy).await.expect("first edge");

    let mut members_policy = row_local_policy(members.clone(), "via_messages", 0);
    members_policy.using_program = Some(membership_program(&members, &messages));
    let error = provider
        .create_policy(members_policy)
        .await
        .expect_err("cycle must fail closed");

    assert!(error.to_string().contains("cycle"));
}

#[tokio::test]
async fn ensure_policy_is_idempotent() {
    let provider = TablePoliciesTableProvider::new(Arc::new(InMemoryBackend::new()));
    let documents = TableId::from_strings("app", "documents_ensure");
    let policy = row_local_policy(documents.clone(), "owner_read", 0);

    let first = provider.ensure_policy(policy.clone()).await.expect("first ensure");
    let second = provider.ensure_policy(policy).await.expect("second ensure");

    assert_eq!(first.policy_generation, second.policy_generation);
    assert_eq!(provider.list_for_table(&documents).unwrap().len(), 1);
}

fn membership_program(protected: &TableId, relation: &TableId) -> PolicyProgram {
    PolicyProgram::AuthorizationRelation(kalamdb_commons::AuthorizationRelation {
        protected_table: protected.clone(),
        protected_keys: vec![2],
        relation_table: relation.clone(),
        relation_keys: vec![2],
        principal_column: 1,
        principal: PrincipalExpr::CurrentUser,
        static_predicates: Vec::new(),
        dependencies: vec![relation.clone()],
        invalidation: kalamdb_commons::InvalidationStrategy::TargetedPrincipal,
    })
}
