use kalamdb_commons::models::rows::Row;

use crate::{AuthorizationPolicyGuard, BoundAuthorization};

/// Subscription-time authorization evaluator used by live fan-out.
#[derive(Debug, Clone)]
pub struct BoundLiveAuthorization {
    authorization: BoundAuthorization,
    policy_guard:  Option<AuthorizationPolicyGuard>,
}

impl BoundLiveAuthorization {
    pub fn new(
        authorization: BoundAuthorization,
        policy_guard: Option<AuthorizationPolicyGuard>,
    ) -> Self {
        Self {
            authorization,
            policy_guard,
        }
    }

    pub fn is_current(&self) -> bool {
        self.policy_guard.as_ref().is_none_or(AuthorizationPolicyGuard::is_current)
            && self.authorization.is_current()
    }

    pub fn authorizes(&self, row: &Row) -> bool {
        if !self.is_current() {
            return false;
        }
        self.authorization.authorizes(row) && self.is_current()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
    };

    use kalamdb_commons::{
        datatypes::KalamDataType,
        schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
        AuthorizationRelation, BoundExprShape, InvalidationStrategy, NamespaceId, PolicyCommand,
        PolicyId, PolicyProgram, PolicyTarget, PrincipalExpr, Role, TableId, TableName,
        TablePolicy, UserId,
    };

    use super::*;
    use crate::{
        AuthorizationDependencyGuard, AuthorizationSet, BoundTablePolicies,
        LiveAuthorizationRoute, MembershipEpoch,
    };

    struct ChangingMembershipEpoch {
        reads: AtomicU64,
    }

    impl MembershipEpoch for ChangingMembershipEpoch {
        fn authorization_generation(&self) -> u64 {
            if self.reads.fetch_add(1, Ordering::Relaxed) < 2 {
                7
            } else {
                8
            }
        }

        fn has_active_authorization_mutations(&self) -> bool {
            false
        }
    }

    fn table() -> Arc<TableDefinition> {
        Arc::new(
            TableDefinition::new(
                NamespaceId::new("app"),
                TableName::new("messages"),
                TableType::Shared,
                vec![ColumnDefinition::primary_key(
                    1,
                    "id",
                    1,
                    KalamDataType::Text,
                ),
                ColumnDefinition::simple(2, "conversation_id", 2, KalamDataType::Text)],
                TableOptions::shared(),
                None,
            )
            .unwrap(),
        )
    }

    fn allow_all_policies() -> BoundTablePolicies {
        let table_id = TableId::from_strings("app", "messages");
        let policy = TablePolicy::new(
            PolicyId::new(table_id.clone(), "allow_all").unwrap(),
            table_id,
            "allow_all",
            PolicyCommand::Select,
            vec![PolicyTarget::Public],
            Some("true".to_string()),
            None,
            Some(PolicyProgram::RowLocal {
                expr: BoundExprShape::Literal(true),
            }),
            None,
            1,
            1,
        );
        BoundTablePolicies::bind(&[policy], UserId::new("alice"), Role::User, PolicyCommand::Select)
    }

    #[test]
    fn membership_change_during_evaluation_fails_closed() {
        let dependency = AuthorizationDependencyGuard::capture(
            TableId::from_strings("app", "members"),
            Arc::new(ChangingMembershipEpoch {
                reads: AtomicU64::new(0),
            }),
        );
        let authorization = BoundLiveAuthorization::new(
            BoundAuthorization::new(
                table(),
                allow_all_policies(),
                HashMap::new(),
                vec![dependency],
            ),
            None,
        );

        assert!(!authorization.authorizes(&Row::from_vec(Vec::new())));
    }

    #[test]
    fn owner_policy_binds_principal_as_live_route_key() {
        let table_id = TableId::from_strings("app", "messages");
        let policy = TablePolicy::new(
            PolicyId::new(table_id.clone(), "owner_read").unwrap(),
            table_id,
            "owner_read",
            PolicyCommand::Select,
            vec![PolicyTarget::Role(Role::User)],
            Some("conversation_id = CURRENT_USER".to_string()),
            None,
            Some(PolicyProgram::RowLocal {
                expr: BoundExprShape::ColumnEqualsPrincipal {
                    column_id: 2,
                    principal: PrincipalExpr::CurrentUser,
                },
            }),
            None,
            1,
            1,
        );
        let policies = BoundTablePolicies::bind(
            &[policy],
            UserId::new("conv-123"),
            Role::User,
            PolicyCommand::Select,
        );
        let authorization = BoundAuthorization::new(table(), policies, HashMap::new(), Vec::new());

        assert_eq!(
            authorization.live_route().unwrap(),
            LiveAuthorizationRoute::Keyed(vec![(
                2,
                datafusion::scalar::ScalarValue::Utf8(Some("conv-123".to_string())),
            )])
        );
    }

    #[test]
    fn membership_policy_binds_authorization_set_as_live_route_keys() {
        let table_id = TableId::from_strings("app", "messages");
        let relation_table = TableId::from_strings("app", "conversation_members");
        let relation = AuthorizationRelation {
            protected_table: table_id.clone(),
            protected_keys: vec![2],
            relation_table: relation_table.clone(),
            relation_keys: vec![3],
            principal_column: 2,
            principal: PrincipalExpr::CurrentUser,
            static_predicates: Vec::new(),
            dependencies: vec![relation_table],
            invalidation: InvalidationStrategy::TargetedPrincipal,
        };
        let policy_id = PolicyId::new(table_id.clone(), "member_read").unwrap();
        let policy = TablePolicy::new(
            policy_id.clone(),
            table_id,
            "member_read",
            PolicyCommand::Select,
            vec![PolicyTarget::Role(Role::User)],
            Some("membership".to_string()),
            None,
            Some(PolicyProgram::AuthorizationRelation(relation.clone())),
            None,
            1,
            1,
        );
        let policies = BoundTablePolicies::bind(
            &[policy],
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Select,
        );
        let set = AuthorizationSet::from_keys(
            relation,
            vec![
                vec![datafusion::scalar::ScalarValue::Utf8(Some("conv-123".to_string()))],
                vec![datafusion::scalar::ScalarValue::Utf8(Some("conv-456".to_string()))],
            ],
        );
        let authorization = BoundAuthorization::new(
            table(),
            policies,
            HashMap::from([(policy_id, Arc::new(set))]),
            Vec::new(),
        );

        assert_eq!(
            authorization.live_route().unwrap(),
            LiveAuthorizationRoute::Keyed(vec![
                (2, datafusion::scalar::ScalarValue::Utf8(Some("conv-123".to_string()))),
                (2, datafusion::scalar::ScalarValue::Utf8(Some("conv-456".to_string()))),
            ])
        );
    }
}
