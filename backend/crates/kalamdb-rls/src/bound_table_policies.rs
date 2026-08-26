use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{
    models::rows::Row, schemas::TableDefinition, BoundExprShape, PolicyCommand, PolicyId,
    PolicyProgram, PolicyScalar, PolicyTarget, PrincipalExpr, Role, TableId, TablePolicy, UserId,
};

use crate::{row_access::*, AuthorizationCacheKey, AuthorizationSet};

#[derive(Debug, Clone)]
pub struct BoundPolicy {
    pub policy_id:         PolicyId,
    pub policy_name:       String,
    pub command:           PolicyCommand,
    pub policy_generation: u64,
    pub program:           PolicyProgram,
    explain_clause:        Option<String>,
}

#[derive(Debug, Clone)]
pub struct BoundTablePolicies {
    principal: UserId,
    bypass:    bool,
    policies:  Vec<BoundPolicy>,
}

impl BoundTablePolicies {
    /// System/DBA: no catalog lookup and no per-row evaluation.
    pub fn admin_bypass(principal: UserId) -> Self {
        Self {
            principal,
            bypass: true,
            policies: Vec::new(),
        }
    }

    /// FORCE RLS with zero applicable policies: deny every row without evaluation.
    pub fn default_deny(principal: UserId) -> Self {
        Self {
            principal,
            bypass: false,
            policies: Vec::new(),
        }
    }

    pub fn bind(
        policies: &[TablePolicy],
        principal: UserId,
        role: Role,
        command: PolicyCommand,
    ) -> Self {
        Self::bind_with_program(policies, principal, role, command, false)
    }

    pub fn bind_check(
        policies: &[TablePolicy],
        principal: UserId,
        role: Role,
        command: PolicyCommand,
    ) -> Self {
        Self::bind_with_program(policies, principal, role, command, true)
    }

    fn bind_with_program(
        policies: &[TablePolicy],
        principal: UserId,
        role: Role,
        command: PolicyCommand,
        check: bool,
    ) -> Self {
        if matches!(role, Role::System | Role::Dba) {
            return Self::admin_bypass(principal);
        }
        if policies.is_empty() {
            return Self::default_deny(principal);
        }
        let policies = policies
            .iter()
            .filter(|policy| policy.targets.iter().any(|target| target_applies(target, role)))
            .filter_map(|policy| {
                let program = if check {
                    policy.check_program_for(command)
                } else {
                    policy.using_program_for(command)
                };
                program.cloned().map(|program| BoundPolicy {
                    policy_id: policy.policy_id.clone(),
                    policy_name: policy.policy_name.clone(),
                    command: policy.command,
                    policy_generation: policy.policy_generation,
                    program,
                    explain_clause: if check {
                        policy
                            .with_check_sql
                            .as_deref()
                            .map(|sql| format_policy_qual("WITH CHECK", sql))
                    } else {
                        policy
                            .using_sql
                            .as_deref()
                            .map(|sql| format_policy_qual("USING", sql))
                    },
                })
            })
            .collect();
        Self {
            principal,
            bypass: false,
            policies,
        }
    }

    pub fn bypasses_rls(&self) -> bool {
        self.bypass
    }

    pub fn is_default_deny(&self) -> bool {
        !self.bypass && self.policies.is_empty()
    }

    /// PostgreSQL-style policy listing for EXPLAIN: names plus USING / WITH CHECK.
    ///
    /// Admin bypass returns `None` (no security quals). Default-deny returns
    /// `policies=[]`. Original policy SQL is used so the bound principal is never printed.
    pub fn explain_policies(&self) -> Option<String> {
        if self.bypass {
            return None;
        }
        if self.policies.is_empty() {
            return Some("policies=[]".to_string());
        }
        let items = self
            .policies
            .iter()
            .map(|policy| {
                let mut item = format!("{} FOR {}", policy.policy_name, policy.command.as_sql());
                if let Some(clause) = &policy.explain_clause {
                    item.push(' ');
                    item.push_str(clause);
                }
                item
            })
            .collect::<Vec<_>>()
            .join("; ");
        Some(format!("policies=[{items}]"))
    }

    pub fn policies(&self) -> &[BoundPolicy] {
        &self.policies
    }

    pub fn authorization_cache_key(
        &self,
        policy: &BoundPolicy,
        principal: &UserId,
        relation_table: &TableId,
        relation_generation: u64,
        snapshot_commit_seq: Option<u64>,
    ) -> AuthorizationCacheKey {
        AuthorizationCacheKey {
            policy_id: policy.policy_id.clone(),
            policy_generation: policy.policy_generation,
            principal_id: principal.clone(),
            relation_table: relation_table.clone(),
            relation_generation,
            snapshot_commit_seq,
        }
    }

    pub fn single_membership_policy(
        &self,
    ) -> Option<(&BoundPolicy, &kalamdb_commons::AuthorizationRelation)> {
        let [policy] = self.policies.as_slice() else {
            return None;
        };
        let PolicyProgram::AuthorizationRelation(relation) = &policy.program else {
            return None;
        };
        Some((policy, relation))
    }

    pub fn principal(&self) -> &UserId {
        &self.principal
    }

    pub fn required_column_ids(&self) -> Vec<u64> {
        let mut column_ids = Vec::new();
        for policy in &self.policies {
            match &policy.program {
                PolicyProgram::RowLocal { expr } => {
                    collect_row_local_column_ids(expr, &mut column_ids)
                },
                PolicyProgram::AuthorizationRelation(relation) => {
                    column_ids.extend(relation.protected_keys.iter().copied());
                },
            }
        }
        column_ids.sort_unstable();
        column_ids.dedup();
        column_ids
    }

    pub fn authorizes_row_with_sets(
        &self,
        table: &TableDefinition,
        row: &Row,
        authorization_sets: &HashMap<PolicyId, Arc<AuthorizationSet>>,
    ) -> bool {
        if self.bypass {
            return true;
        }
        self.policies.iter().any(|policy| match &policy.program {
            PolicyProgram::RowLocal { expr } => {
                evaluate_row_local(expr, table, row, &self.principal) == SqlTruth::True
            },
            PolicyProgram::AuthorizationRelation(_) => authorization_sets
                .get(&policy.policy_id)
                .is_some_and(|set| set.contains_protected_row(table, row)),
        })
    }
}

fn collect_row_local_column_ids(expression: &BoundExprShape, column_ids: &mut Vec<u64>) {
    match expression {
        BoundExprShape::ColumnEqualsPrincipal { column_id, .. }
        | BoundExprShape::ColumnEqualsScalar { column_id, .. } => column_ids.push(*column_id),
        BoundExprShape::And(expressions) | BoundExprShape::Or(expressions) => {
            for expression in expressions {
                collect_row_local_column_ids(expression, column_ids);
            }
        },
        BoundExprShape::Not(expression) => collect_row_local_column_ids(expression, column_ids),
        BoundExprShape::Literal(_) => {},
    }
}

fn format_policy_qual(label: &str, sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.starts_with('(') && trimmed.ends_with(')') {
        format!("{label} {trimmed}")
    } else {
        format!("{label} ({trimmed})")
    }
}

fn target_applies(target: &PolicyTarget, role: Role) -> bool {
    matches!(target, PolicyTarget::Public)
        || matches!(target, PolicyTarget::Role(target_role) if *target_role == role)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SqlTruth {
    True,
    False,
    Unknown,
}

impl SqlTruth {
    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

fn evaluate_row_local(
    expression: &BoundExprShape,
    table: &TableDefinition,
    row: &Row,
    principal: &UserId,
) -> SqlTruth {
    match expression {
        BoundExprShape::Literal(true) => SqlTruth::True,
        BoundExprShape::Literal(false) => SqlTruth::False,
        BoundExprShape::ColumnEqualsPrincipal {
            column_id,
            principal: principal_expr,
        } => {
            if !matches!(principal_expr, PrincipalExpr::CurrentUser) {
                return SqlTruth::False;
            }
            let Some(value) = column_value(table, row, *column_id) else {
                return SqlTruth::Unknown;
            };
            if value.is_null() {
                SqlTruth::Unknown
            } else if scalar_equals_principal(value, principal) {
                SqlTruth::True
            } else {
                SqlTruth::False
            }
        },
        BoundExprShape::ColumnEqualsScalar { column_id, value } => {
            let Some(row_value) = column_value(table, row, *column_id) else {
                return SqlTruth::Unknown;
            };
            if row_value.is_null() || matches!(value, PolicyScalar::Null) {
                SqlTruth::Unknown
            } else if scalar_equals_policy(row_value, value) {
                SqlTruth::True
            } else {
                SqlTruth::False
            }
        },
        BoundExprShape::And(expressions) => {
            let mut result = SqlTruth::True;
            for expression in expressions {
                match evaluate_row_local(expression, table, row, principal) {
                    SqlTruth::False => return SqlTruth::False,
                    SqlTruth::Unknown => result = SqlTruth::Unknown,
                    SqlTruth::True => {},
                }
            }
            result
        },
        BoundExprShape::Or(expressions) => {
            let mut result = SqlTruth::False;
            for expression in expressions {
                match evaluate_row_local(expression, table, row, principal) {
                    SqlTruth::True => return SqlTruth::True,
                    SqlTruth::Unknown => result = SqlTruth::Unknown,
                    SqlTruth::False => {},
                }
            }
            result
        },
        BoundExprShape::Not(expression) => {
            evaluate_row_local(expression, table, row, principal).not()
        },
    }
}

#[cfg(test)]
mod tests {
    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        datatypes::KalamDataType,
        models::rows::Row,
        schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
        AuthorizationRelation, BoundExprShape, InvalidationStrategy, NamespaceId, PolicyCommand,
        PolicyId, PolicyProgram, PolicyTarget, PrincipalExpr, Role, TableId, TableName,
        TablePolicy, UserId,
    };

    use super::{AuthorizationSet, BoundTablePolicies};

    fn empty_sets(
    ) -> std::collections::HashMap<kalamdb_commons::PolicyId, std::sync::Arc<AuthorizationSet>>
    {
        std::collections::HashMap::new()
    }

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
            None,
        )
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
            1,
        )
    }

    fn row(owner_id: &str) -> Row {
        Row::from_vec(vec![("owner_id".to_string(), ScalarValue::Utf8(Some(owner_id.to_string())))])
    }

    #[test]
    fn subjects_are_default_denied_but_admins_bypass() {
        let user =
            BoundTablePolicies::bind(&[], UserId::new("alice"), Role::User, PolicyCommand::Select);
        assert!(!user.authorizes_row_with_sets(&table(), &row("alice"), &empty_sets()));
        assert!(user.is_default_deny());

        let dba = BoundTablePolicies::admin_bypass(UserId::new("root"));
        assert!(dba.authorizes_row_with_sets(&table(), &row("alice"), &empty_sets()));
        assert!(dba.bypasses_rls());
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
            },
        );
        let bound = BoundTablePolicies::bind(
            &[owner],
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Select,
        );

        assert!(bound.authorizes_row_with_sets(&table(), &row("alice"), &empty_sets()));
        assert!(!bound.authorizes_row_with_sets(&table(), &row("bob"), &empty_sets()));
    }

    #[test]
    fn policy_targets_filter_before_permissive_or() {
        let service_only = policy(
            "service_read",
            PolicyTarget::Role(Role::Service),
            PolicyProgram::RowLocal {
                expr: BoundExprShape::Literal(true),
            },
        );
        let bound = BoundTablePolicies::bind(
            &[service_only],
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Select,
        );

        assert!(!bound.authorizes_row_with_sets(&table(), &row("alice"), &empty_sets()));
    }

    #[test]
    fn membership_policy_authorizes_only_keys_in_bound_set() {
        let relation = AuthorizationRelation {
            protected_table:   TableId::from_strings("app", "documents"),
            protected_keys:    vec![1],
            relation_table:    TableId::from_strings("app", "document_members"),
            relation_keys:     vec![2],
            principal_column:  1,
            principal:         PrincipalExpr::CurrentUser,
            static_predicates: Vec::new(),
            dependencies:      vec![TableId::from_strings("app", "document_members")],
            invalidation:      InvalidationStrategy::TargetedPrincipal,
        };
        let membership = policy(
            "member_read",
            PolicyTarget::Public,
            PolicyProgram::AuthorizationRelation(relation.clone()),
        );
        let bound = BoundTablePolicies::bind(
            std::slice::from_ref(&membership),
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Select,
        );
        let authorization_set = AuthorizationSet::from_keys(
            relation,
            vec![vec![ScalarValue::Utf8(Some("doc-1".to_string()))]],
        );
        let sets = std::collections::HashMap::from([(
            membership.policy_id,
            std::sync::Arc::new(authorization_set),
        )]);
        let allowed =
            Row::from_vec(vec![("id".to_string(), ScalarValue::Utf8(Some("doc-1".to_string())))]);
        let denied =
            Row::from_vec(vec![("id".to_string(), ScalarValue::Utf8(Some("doc-2".to_string())))]);

        assert!(bound.authorizes_row_with_sets(&table(), &allowed, &sets));
        assert!(!bound.authorizes_row_with_sets(&table(), &denied, &sets));
    }

    #[test]
    fn not_of_null_comparison_does_not_authorize_row() {
        let owner_is_not_current = policy(
            "not_owner",
            PolicyTarget::Public,
            PolicyProgram::RowLocal {
                expr: BoundExprShape::Not(Box::new(BoundExprShape::ColumnEqualsPrincipal {
                    column_id: 2,
                    principal: PrincipalExpr::CurrentUser,
                })),
            },
        );
        let bound = BoundTablePolicies::bind(
            &[owner_is_not_current],
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Select,
        );
        let null_owner = Row::from_vec(vec![("owner_id".to_string(), ScalarValue::Utf8(None))]);

        assert!(!bound.authorizes_row_with_sets(&table(), &null_owner, &empty_sets()));
    }

    #[test]
    fn explain_policies_lists_using_sql_without_bound_principal() {
        let owner = policy(
            "owner_read",
            PolicyTarget::Role(Role::User),
            PolicyProgram::RowLocal {
                expr: BoundExprShape::ColumnEqualsPrincipal {
                    column_id: 2,
                    principal: PrincipalExpr::CurrentUser,
                },
            },
        );
        let bound = BoundTablePolicies::bind(
            &[owner],
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Select,
        );

        let explain = bound.explain_policies().expect("user policies appear in EXPLAIN");
        assert_eq!(
            explain,
            "policies=[owner_read FOR SELECT USING (owner_id = CURRENT_USER)]"
        );
        assert!(!explain.contains("alice"));
    }

    #[test]
    fn explain_policies_shows_with_check_for_insert_bind() {
        let table_id = TableId::from_strings("app", "documents");
        let insert = TablePolicy::new(
            PolicyId::new(table_id.clone(), "owner_write").unwrap(),
            table_id,
            "owner_write",
            PolicyCommand::Insert,
            vec![PolicyTarget::Public],
            None,
            Some("owner_id = CURRENT_USER".to_string()),
            None,
            Some(PolicyProgram::RowLocal {
                expr: BoundExprShape::ColumnEqualsPrincipal {
                    column_id: 2,
                    principal: PrincipalExpr::CurrentUser,
                },
            }),
            1,
            1,
        );
        let bound = BoundTablePolicies::bind_check(
            &[insert],
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Insert,
        );

        assert_eq!(
            bound.explain_policies().as_deref(),
            Some("policies=[owner_write FOR INSERT WITH CHECK (owner_id = CURRENT_USER)]")
        );
    }

    #[test]
    fn explain_policies_omits_admin_bypass_and_lists_empty_default_deny() {
        let deny = BoundTablePolicies::bind(
            &[],
            UserId::new("alice"),
            Role::User,
            PolicyCommand::Select,
        );
        assert_eq!(deny.explain_policies().as_deref(), Some("policies=[]"));

        let admin = BoundTablePolicies::admin_bypass(UserId::new("root"));
        assert_eq!(admin.explain_policies(), None);
    }
}
