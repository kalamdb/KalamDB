use std::collections::HashSet;

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    models::rows::Row, schemas::TableDefinition, AuthorizationRelation, PolicyScalar,
    PredicateOperator, ScalarPredicate, UserId,
};

#[derive(Debug, Clone)]
pub struct AuthorizationSet {
    relation: AuthorizationRelation,
    keys:     HashSet<Vec<ScalarValue>>,
}

impl AuthorizationSet {
    pub fn from_keys(relation: AuthorizationRelation, keys: Vec<Vec<ScalarValue>>) -> Self {
        Self {
            relation,
            keys: keys.into_iter().collect(),
        }
    }

    pub fn from_relation_rows<'a>(
        relation: AuthorizationRelation,
        relation_table: &TableDefinition,
        principal: &UserId,
        rows: impl IntoIterator<Item = &'a Row>) -> Self {
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
        let key = self
            .relation
            .protected_keys
            .iter()
            .map(|column_id| {
                let column = table.columns.iter().find(|column| column.column_id == *column_id)?;
                row.values.get(&column.column_name).cloned()
            })
            .collect::<Option<Vec<_>>>();
        key.is_some_and(|key| self.keys.contains(&key))
    }

    pub fn contains_key(&self, key: &[ScalarValue]) -> bool {
        self.keys.contains(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &[ScalarValue]> {
        self.keys.iter().map(|key| key.as_slice())
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

fn relation_row_applies(
    relation: &AuthorizationRelation,
    table: &TableDefinition,
    row: &Row,
    principal: &UserId) -> bool {
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
    predicate: &ScalarPredicate) -> bool {
    let equals = column_value(table, row, predicate.column_id)
        .is_some_and(|value| scalar_equals_policy(value, &predicate.value));
    match predicate.operator {
        PredicateOperator::Eq => equals,
        PredicateOperator::NotEq => !equals,
    }
}

fn column_value<'a>(
    table: &TableDefinition,
    row: &'a Row,
    column_id: u64) -> Option<&'a ScalarValue> {
    let column = table.columns.iter().find(|column| column.column_id == column_id)?;
    row.values.get(&column.column_name)
}

fn scalar_equals_principal(value: &ScalarValue, principal: &UserId) -> bool {
    match value {
        ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => {
            value == principal.as_str()
        },
        _ => false,
    }
}

fn scalar_equals_policy(value: &ScalarValue, expected: &PolicyScalar) -> bool {
    match (value, expected) {
        (value, PolicyScalar::Null) => value.is_null(),
        (ScalarValue::Boolean(Some(value)), PolicyScalar::Boolean(expected)) => value == expected,
        (ScalarValue::Int64(Some(value)), PolicyScalar::Int64(expected)) => value == expected,
        (ScalarValue::UInt64(Some(value)), PolicyScalar::UInt64(expected)) => value == expected,
        (ScalarValue::Float64(Some(value)), PolicyScalar::Float64(expected)) => value == expected,
        (ScalarValue::Utf8(Some(value)), PolicyScalar::String(expected))
        | (ScalarValue::LargeUtf8(Some(value)), PolicyScalar::String(expected)) => {
            value == expected
        },
        _ => false,
    }
}
