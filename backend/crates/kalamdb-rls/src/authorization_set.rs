use std::collections::HashSet;

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    models::rows::Row, schemas::TableDefinition, AuthorizationRelation, PolicyScalar,
    PredicateOperator, ScalarPredicate, UserId,
};

use crate::row_access::{column_value, scalar_equals_policy, scalar_equals_principal};

#[derive(Debug, Clone)]
pub struct AuthorizationSet {
    relation: AuthorizationRelation,
    keys:     AuthorizationKeys,
}

#[derive(Debug, Clone)]
enum AuthorizationKeys {
    Single(HashSet<ScalarValue>),
    Composite(HashSet<Vec<ScalarValue>>),
}

impl AuthorizationSet {
    pub fn from_keys(relation: AuthorizationRelation, keys: Vec<Vec<ScalarValue>>) -> Self {
        let keys = if relation.relation_keys.len() == 1 {
            AuthorizationKeys::Single(
                keys.into_iter()
                    .filter_map(|mut key| {
                        (key.len() == 1 && !key[0].is_null()).then(|| key.swap_remove(0))
                    })
                    .collect(),
            )
        } else {
            AuthorizationKeys::Composite(
                keys.into_iter().filter(|key| !key.iter().any(ScalarValue::is_null)).collect(),
            )
        };
        Self { relation, keys }
    }

    pub fn from_relation_rows<'a>(
        relation: AuthorizationRelation,
        relation_table: &TableDefinition,
        principal: &UserId,
        rows: impl IntoIterator<Item = &'a Row>,
    ) -> Self {
        if let [column_id] = relation.relation_keys.as_slice() {
            let keys = rows
                .into_iter()
                .filter(|row| relation_row_applies(&relation, relation_table, row, principal))
                .filter_map(|row| column_value(relation_table, row, *column_id).cloned())
                .filter(|value| !value.is_null())
                .collect();
            return Self {
                relation,
                keys: AuthorizationKeys::Single(keys),
            };
        }

        let keys = rows
            .into_iter()
            .filter(|row| relation_row_applies(&relation, relation_table, row, principal))
            .filter_map(|row| {
                relation
                    .relation_keys
                    .iter()
                    .map(|column_id| column_value(relation_table, row, *column_id).cloned())
                    .collect::<Option<Vec<_>>>()
            })
            .collect();
        Self::from_keys(relation, keys)
    }

    pub fn contains_protected_row(&self, table: &TableDefinition, row: &Row) -> bool {
        if let [column_id] = self.relation.protected_keys.as_slice() {
            return column_value(table, row, *column_id).is_some_and(|value| {
                !value.is_null() && self.contains_key(std::slice::from_ref(value))
            });
        }

        let key = self
            .relation
            .protected_keys
            .iter()
            .map(|column_id| column_value(table, row, *column_id).cloned())
            .collect::<Option<Vec<_>>>();
        key.is_some_and(|key| self.contains_key(&key))
    }

    pub fn contains_key(&self, key: &[ScalarValue]) -> bool {
        if key.iter().any(ScalarValue::is_null) {
            return false;
        }
        match &self.keys {
            AuthorizationKeys::Single(keys) => {
                let [value] = key else {
                    return false;
                };
                keys.contains(value)
            },
            AuthorizationKeys::Composite(keys) => keys.contains(key),
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.keys {
            AuthorizationKeys::Single(keys) => keys.is_empty(),
            AuthorizationKeys::Composite(keys) => keys.is_empty(),
        }
    }
}

fn relation_row_applies(
    relation: &AuthorizationRelation,
    table: &TableDefinition,
    row: &Row,
    principal: &UserId,
) -> bool {
    column_value(table, row, relation.principal_column)
        .is_some_and(|value| scalar_equals_principal(value, principal))
        && relation
            .static_predicates
            .iter()
            .all(|predicate| static_predicate_matches(table, row, predicate))
}

fn static_predicate_matches(
    table: &TableDefinition,
    row: &Row,
    predicate: &ScalarPredicate,
) -> bool {
    let Some(value) = column_value(table, row, predicate.column_id) else {
        return false;
    };
    if value.is_null() || matches!(predicate.value, PolicyScalar::Null) {
        return false;
    }
    let equals = scalar_equals_policy(value, &predicate.value);
    match predicate.operator {
        PredicateOperator::Eq => equals,
        PredicateOperator::NotEq => !equals,
    }
}

#[cfg(test)]
mod tests {
    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        datatypes::KalamDataType,
        schemas::{ColumnDefinition, TableOptions, TableType},
        InvalidationStrategy, NamespaceId, PolicyScalar, PrincipalExpr, TableId, TableName,
    };

    use super::*;

    fn membership_table() -> TableDefinition {
        TableDefinition::new(
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
        .unwrap()
    }

    fn relation_with_status_not_equal() -> AuthorizationRelation {
        let relation_table = TableId::from_strings("app", "members");
        AuthorizationRelation {
            protected_table:   TableId::from_strings("app", "messages"),
            protected_keys:    vec![2],
            relation_table:    relation_table.clone(),
            relation_keys:     vec![2],
            principal_column:  1,
            principal:         PrincipalExpr::CurrentUser,
            static_predicates: vec![ScalarPredicate {
                column_id: 3,
                operator:  PredicateOperator::NotEq,
                value:     PolicyScalar::String("blocked".to_string()),
            }],
            dependencies:      vec![relation_table],
            invalidation:      InvalidationStrategy::TargetedPrincipal,
        }
    }

    #[test]
    fn null_does_not_satisfy_not_equal_membership_predicate() {
        let row = Row::from_vec(vec![
            ("user_id".to_string(), ScalarValue::Utf8(Some("alice".to_string()))),
            ("group_id".to_string(), ScalarValue::Utf8(Some("group-a".to_string()))),
            ("status".to_string(), ScalarValue::Utf8(None)),
        ]);

        let set = AuthorizationSet::from_relation_rows(
            relation_with_status_not_equal(),
            &membership_table(),
            &UserId::new("alice"),
            [&row],
        );

        assert!(set.is_empty());
    }

    #[test]
    fn null_membership_key_does_not_authorize_null_protected_key() {
        let row = Row::from_vec(vec![
            ("user_id".to_string(), ScalarValue::Utf8(Some("alice".to_string()))),
            ("group_id".to_string(), ScalarValue::Utf8(None)),
            ("status".to_string(), ScalarValue::Utf8(Some("active".to_string()))),
        ]);
        let set = AuthorizationSet::from_relation_rows(
            relation_with_status_not_equal(),
            &membership_table(),
            &UserId::new("alice"),
            [&row],
        );

        assert!(set.is_empty());
        assert!(!set.contains_key(&[ScalarValue::Utf8(None)]));
    }
}
