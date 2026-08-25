//! PostgreSQL-inspired RLS security regression tests.
//!
//! Mirrors scenarios from PostgreSQL's `rowsecurity.sql` and boolean three-valued logic:
//! unknown (NULL) must never widen access, `NOT unknown` stays unknown, and permissive OR
//! only grants when at least one policy is provably true.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    datatypes::KalamDataType,
    models::rows::Row,
    schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
    AuthorizationRelation, BoundExprShape, InvalidationStrategy, NamespaceId, PolicyCommand,
    PolicyId, PolicyProgram, PolicyScalar, PolicyTarget, PredicateOperator, PrincipalExpr, Role,
    ScalarPredicate, TableId, TableName, TablePolicy, UserId,
};

use crate::{
    AuthorizationDependencyGuard, AuthorizationSet, BoundAuthorization, BoundTablePolicies,
    MembershipEpoch,
};

fn documents_table() -> TableDefinition {
    TableDefinition::new(
        NamespaceId::new("app"),
        TableName::new("documents"),
        TableType::Shared,
        vec![
            ColumnDefinition::primary_key(1, "id", 1, KalamDataType::Text),
            ColumnDefinition::simple(2, "owner_id", 2, KalamDataType::Text),
            ColumnDefinition::simple(3, "status", 3, KalamDataType::Text),
        ],
        TableOptions::shared(),
        None,
    )
    .unwrap()
}

fn row(owner_id: Option<&str>, status: Option<&str>) -> Row {
    Row::from_vec(vec![
        ("owner_id".to_string(), ScalarValue::Utf8(owner_id.map(str::to_string))),
        ("status".to_string(), ScalarValue::Utf8(status.map(str::to_string))),
    ])
}

fn policy(name: &str, program: PolicyProgram) -> TablePolicy {
    let table_id = TableId::from_strings("app", "documents");
    TablePolicy::new(
        PolicyId::new(table_id.clone(), name).unwrap(),
        table_id,
        name,
        PolicyCommand::Select,
        vec![PolicyTarget::Public],
        None,
        None,
        Some(program),
        None,
        1,
        1,
    )
}

fn bind(program: PolicyProgram) -> BoundTablePolicies {
    BoundTablePolicies::bind(
        &[policy("p", program)],
        UserId::new("alice"),
        Role::User,
        PolicyCommand::Select,
    )
}

fn authorizes(bound: &BoundTablePolicies, row: &Row) -> bool {
    bound.authorizes_row_with_sets(&documents_table(), row, &HashMap::new())
}

fn owner_equals_current_user() -> BoundExprShape {
    BoundExprShape::ColumnEqualsPrincipal {
        column_id: 2,
        principal: PrincipalExpr::CurrentUser,
    }
}

#[test]
fn null_owner_does_not_match_current_user() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: owner_equals_current_user(),
    });
    assert!(!authorizes(&bound, &row(None, Some("active"))));
}

#[test]
fn not_of_null_equality_does_not_authorize() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::Not(Box::new(owner_equals_current_user())),
    });
    assert!(!authorizes(&bound, &row(None, Some("active"))));
}

#[test]
fn double_not_of_null_equality_stays_unknown_and_denies() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::Not(Box::new(BoundExprShape::Not(Box::new(
            owner_equals_current_user(),
        )))),
    });
    assert!(!authorizes(&bound, &row(None, Some("active"))));
}

#[test]
fn not_of_non_matching_owner_authorizes() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::Not(Box::new(owner_equals_current_user())),
    });
    assert!(authorizes(&bound, &row(Some("bob"), Some("active"))));
}

#[test]
fn not_of_matching_owner_denies() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::Not(Box::new(owner_equals_current_user())),
    });
    assert!(!authorizes(&bound, &row(Some("alice"), Some("active"))));
}

#[test]
fn or_false_with_unknown_denies() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::Or(vec![BoundExprShape::Literal(false), owner_equals_current_user()]),
    });
    assert!(!authorizes(&bound, &row(None, Some("active"))));
}

#[test]
fn or_true_with_unknown_authorizes() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::Or(vec![BoundExprShape::Literal(true), owner_equals_current_user()]),
    });
    assert!(authorizes(&bound, &row(None, Some("active"))));
}

#[test]
fn and_true_with_unknown_denies() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::And(vec![BoundExprShape::Literal(true), owner_equals_current_user()]),
    });
    assert!(!authorizes(&bound, &row(None, Some("active"))));
}

#[test]
fn and_false_with_unknown_denies_via_short_circuit() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::And(vec![
            BoundExprShape::Literal(false),
            owner_equals_current_user(),
        ]),
    });
    assert!(!authorizes(&bound, &row(Some("alice"), Some("active"))));
}

#[test]
fn null_scalar_equality_is_unknown_and_denies() {
    let bound = bind(PolicyProgram::RowLocal {
        expr: BoundExprShape::ColumnEqualsScalar {
            column_id: 3,
            value:     PolicyScalar::String("active".to_string()),
        },
    });
    assert!(!authorizes(&bound, &row(Some("alice"), None)));
}

#[test]
fn null_membership_key_does_not_authorize_protected_row() {
    let relation = AuthorizationRelation {
        protected_table:   TableId::from_strings("app", "documents"),
        protected_keys:    vec![1],
        relation_table:    TableId::from_strings("app", "members"),
        relation_keys:     vec![2],
        principal_column:  1,
        principal:         PrincipalExpr::CurrentUser,
        static_predicates: Vec::new(),
        dependencies:      vec![TableId::from_strings("app", "members")],
        invalidation:      InvalidationStrategy::TargetedPrincipal,
    };
    let membership = policy("member_read", PolicyProgram::AuthorizationRelation(relation.clone()));
    let bound = BoundTablePolicies::bind(
        &[membership.clone()],
        UserId::new("alice"),
        Role::User,
        PolicyCommand::Select,
    );
    let authorization_set = AuthorizationSet::from_keys(
        relation,
        vec![vec![ScalarValue::Utf8(Some("doc-1".to_string()))]],
    );
    let sets = HashMap::from([(membership.policy_id, Arc::new(authorization_set))]);
    let null_key_row = Row::from_vec(vec![("id".to_string(), ScalarValue::Utf8(None))]);
    assert!(!bound.authorizes_row_with_sets(&documents_table(), &null_key_row, &sets));
}

#[test]
fn null_principal_membership_row_is_ignored() {
    let relation_table = TableId::from_strings("app", "members");
    let relation = AuthorizationRelation {
        protected_table:   TableId::from_strings("app", "messages"),
        protected_keys:    vec![2],
        relation_table:    relation_table.clone(),
        relation_keys:     vec![2],
        principal_column:  1,
        principal:         PrincipalExpr::CurrentUser,
        static_predicates: Vec::new(),
        dependencies:      vec![relation_table],
        invalidation:      InvalidationStrategy::TargetedPrincipal,
    };
    let members = TableDefinition::new(
        NamespaceId::new("app"),
        TableName::new("members"),
        TableType::Shared,
        vec![
            ColumnDefinition::primary_key(1, "user_id", 1, KalamDataType::Text),
            ColumnDefinition::simple(2, "group_id", 2, KalamDataType::Text),
        ],
        TableOptions::shared(),
        None,
    )
    .unwrap();
    let membership_row = Row::from_vec(vec![
        ("user_id".to_string(), ScalarValue::Utf8(None)),
        ("group_id".to_string(), ScalarValue::Utf8(Some("group-a".to_string()))),
    ]);
    let set = AuthorizationSet::from_relation_rows(
        relation,
        &members,
        &UserId::new("alice"),
        [&membership_row],
    );
    assert!(set.is_empty());
}

#[test]
fn equality_static_predicate_rejects_null_column() {
    let relation_table = TableId::from_strings("app", "members");
    let relation = AuthorizationRelation {
        protected_table:   TableId::from_strings("app", "messages"),
        protected_keys:    vec![2],
        relation_table:    relation_table.clone(),
        relation_keys:     vec![2],
        principal_column:  1,
        principal:         PrincipalExpr::CurrentUser,
        static_predicates: vec![ScalarPredicate {
            column_id: 3,
            operator:  PredicateOperator::Eq,
            value:     PolicyScalar::String("active".to_string()),
        }],
        dependencies:      vec![relation_table],
        invalidation:      InvalidationStrategy::TargetedPrincipal,
    };
    let members = TableDefinition::new(
        NamespaceId::new("app"),
        TableName::new("members"),
        TableType::Shared,
        vec![
            ColumnDefinition::primary_key(1, "user_id", 1, KalamDataType::Text),
            ColumnDefinition::simple(2, "group_id", 2, KalamDataType::Text),
            ColumnDefinition::simple(3, "status", 3, KalamDataType::Text),
        ],
        TableOptions::shared(),
        None,
    )
    .unwrap();
    let membership_row = Row::from_vec(vec![
        ("user_id".to_string(), ScalarValue::Utf8(Some("alice".to_string()))),
        ("group_id".to_string(), ScalarValue::Utf8(Some("group-a".to_string()))),
        ("status".to_string(), ScalarValue::Utf8(None)),
    ]);
    let set = AuthorizationSet::from_relation_rows(
        relation,
        &members,
        &UserId::new("alice"),
        [&membership_row],
    );
    assert!(set.is_empty());
}

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

fn allow_all_policies() -> BoundTablePolicies {
    let allow = policy(
        "allow_all",
        PolicyProgram::RowLocal {
            expr: BoundExprShape::Literal(true),
        },
    );
    BoundTablePolicies::bind(&[allow], UserId::new("alice"), Role::User, PolicyCommand::Select)
}

#[test]
fn batch_authorization_fails_closed_when_membership_epoch_changes() {
    let dependency = AuthorizationDependencyGuard::capture(
        TableId::from_strings("app", "members"),
        Arc::new(ChangingMembershipEpoch {
            reads: AtomicU64::new(0),
        }),
    );
    let authorization = BoundAuthorization::new(
        Arc::new(documents_table()),
        allow_all_policies(),
        HashMap::new(),
        vec![dependency],
    );
    let rows = [row(Some("alice"), Some("active"))];
    assert!(!authorization.authorizes_all(&rows));
}

#[test]
fn filter_authorized_returns_none_when_epoch_changes_mid_batch() {
    let dependency = AuthorizationDependencyGuard::capture(
        TableId::from_strings("app", "members"),
        Arc::new(ChangingMembershipEpoch {
            reads: AtomicU64::new(0),
        }),
    );
    let authorization = BoundAuthorization::new(
        Arc::new(documents_table()),
        allow_all_policies(),
        HashMap::new(),
        vec![dependency],
    );
    let rows = vec![row(Some("alice"), Some("active"))];
    assert!(authorization.filter_authorized(rows, |item| item).is_none());
}

#[test]
fn permissive_or_across_policies_grants_when_any_policy_matches() {
    let owner_policy = policy(
        "owner_read",
        PolicyProgram::RowLocal {
            expr: owner_equals_current_user(),
        },
    );
    let public_policy = policy(
        "public_active",
        PolicyProgram::RowLocal {
            expr: BoundExprShape::ColumnEqualsScalar {
                column_id: 3,
                value:     PolicyScalar::String("public".to_string()),
            },
        },
    );
    let bound = BoundTablePolicies::bind(
        &[owner_policy, public_policy],
        UserId::new("alice"),
        Role::User,
        PolicyCommand::Select,
    );
    assert!(authorizes(&bound, &row(Some("bob"), Some("public"))));
    assert!(!authorizes(&bound, &row(Some("bob"), Some("private"))));
}
