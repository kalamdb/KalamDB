mod authorization_relation;
mod bound_expr_shape;
mod invalidation_strategy;
mod policy_command;
mod policy_id;
mod policy_program;
mod policy_scalar;
mod policy_target;
mod principal_expr;
mod scalar_predicate;
mod table_policy;

pub use authorization_relation::AuthorizationRelation;
pub use bound_expr_shape::BoundExprShape;
pub use invalidation_strategy::InvalidationStrategy;
pub use policy_command::PolicyCommand;
pub use policy_id::PolicyId;
pub use policy_program::PolicyProgram;
pub use policy_scalar::PolicyScalar;
pub use policy_target::PolicyTarget;
pub use principal_expr::PrincipalExpr;
pub use scalar_predicate::{PredicateOperator, ScalarPredicate};
pub use table_policy::TablePolicy;

#[cfg(test)]
mod tests {
    use crate::{Role, TableId, UserId};

    use super::{
        AuthorizationRelation, BoundExprShape, InvalidationStrategy, PolicyCommand, PolicyId,
        PolicyProgram, PolicyTarget, PrincipalExpr, TablePolicy,
    };

    #[test]
    fn table_policy_serialization_preserves_compiled_authorization_metadata() {
        let table_id = TableId::from_strings("chat", "messages");
        let relation_id = TableId::from_strings("chat", "group_members");
        let policy = TablePolicy::new(
            PolicyId::new(table_id.clone(), "member_read").expect("valid policy id"),
            table_id.clone(),
            "member_read",
            PolicyCommand::Select,
            vec![PolicyTarget::Role(Role::User), PolicyTarget::Role(Role::Service)],
            Some("group_id IN (SELECT group_id FROM chat.group_members WHERE user_id = CURRENT_USER)".to_string()),
            None,
            Some(PolicyProgram::AuthorizationRelation(AuthorizationRelation {
                protected_table: table_id,
                protected_keys: vec![2],
                relation_table: relation_id.clone(),
                relation_keys: vec![2],
                principal_column: 1,
                principal: PrincipalExpr::CurrentUser,
                static_predicates: Vec::new(),
                dependencies: vec![relation_id],
                invalidation: InvalidationStrategy::TargetedPrincipal,
            })),
            None,
            7,
            3,
        );

        let encoded = serde_json::to_string(&policy).expect("serialize policy");
        let decoded: TablePolicy = serde_json::from_str(&encoded).expect("deserialize policy");

        assert_eq!(decoded, policy);
        assert_eq!(decoded.policy_generation, 7);
        assert_eq!(decoded.schema_generation, 3);
    }

    #[test]
    fn policy_targets_match_public_or_the_bound_role() {
        assert!(PolicyTarget::Public.matches(Role::User));
        assert!(PolicyTarget::Public.matches(Role::Service));
        assert!(PolicyTarget::Role(Role::User).matches(Role::User));
        assert!(!PolicyTarget::Role(Role::User).matches(Role::Service));
    }

    #[test]
    fn table_policy_selects_program_by_command_semantics() {
        let table_id = TableId::from_strings("app", "documents");
        let using_program = PolicyProgram::RowLocal {
            expr: BoundExprShape::ColumnEqualsPrincipal {
                column_id: 2,
                principal: PrincipalExpr::CurrentUser,
            },
        };
        let check_program = PolicyProgram::RowLocal {
            expr: BoundExprShape::Literal(true),
        };
        let policy = TablePolicy::new(
            PolicyId::new(table_id.clone(), "owner_all").expect("valid policy id"),
            table_id,
            "owner_all",
            PolicyCommand::All,
            vec![PolicyTarget::Public],
            Some("owner_id = CURRENT_USER".to_string()),
            Some("true".to_string()),
            Some(using_program.clone()),
            Some(check_program.clone()),
            1,
            1,
        );

        assert_eq!(policy.using_program_for(PolicyCommand::Select), Some(&using_program));
        assert_eq!(policy.using_program_for(PolicyCommand::Delete), Some(&using_program));
        assert_eq!(policy.check_program_for(PolicyCommand::Insert), Some(&check_program));
        assert_eq!(policy.check_program_for(PolicyCommand::Update), Some(&check_program));
        assert_eq!(policy.using_program_for(PolicyCommand::Insert), None);
        assert_eq!(policy.check_program_for(PolicyCommand::Select), None);
    }

    #[test]
    fn policy_id_validates_storage_key_shape() {
        let table_id = TableId::from_strings("app", "documents");
        let id = PolicyId::new(table_id, "owner_read").expect("valid policy id");

        assert_eq!(id.as_str(), "app.documents:owner_read");
        assert!(PolicyId::from_parts("app.documents", "bad:name").is_err());

        let principal = UserId::new("alice");
        assert_eq!(principal.as_str(), "alice");
    }
}
