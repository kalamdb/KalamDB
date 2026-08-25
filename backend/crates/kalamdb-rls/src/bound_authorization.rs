use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use kalamdb_commons::{
    conversions::parse_string_as_scalar,
    models::{datatypes::ToArrowType, rows::Row},
    schemas::TableDefinition,
    BoundExprShape, PolicyId, PolicyProgram, PolicyScalar, PrincipalExpr,
};

use crate::{
    AuthorizationDependencyGuard, AuthorizationSet, BoundTablePolicies, LiveAuthorizationRoute,
};

/// Principal-bound policy evaluator shared by SQL, DML, catch-up, and live authorization.
///
/// Dependency epochs are checked on both sides of evaluation. This is a seqlock-style security
/// check: a membership change racing an evaluation invalidates the whole result.
#[derive(Debug, Clone)]
pub struct BoundAuthorization {
    table:        Arc<TableDefinition>,
    policies:     BoundTablePolicies,
    sets:         HashMap<PolicyId, Arc<AuthorizationSet>>,
    dependencies: Vec<AuthorizationDependencyGuard>,
}

impl BoundAuthorization {
    pub fn new(
        table: Arc<TableDefinition>,
        policies: BoundTablePolicies,
        sets: HashMap<PolicyId, Arc<AuthorizationSet>>,
        dependencies: Vec<AuthorizationDependencyGuard>,
    ) -> Self {
        Self {
            table,
            policies,
            sets,
            dependencies,
        }
    }

    pub fn is_current(&self) -> bool {
        self.dependencies.iter().all(AuthorizationDependencyGuard::is_current)
    }

    pub fn authorizes(&self, row: &Row) -> bool {
        self.is_current()
            && self.policies.authorizes_row_with_sets(&self.table, row, &self.sets)
            && self.is_current()
    }

    /// Derive bounded candidate keys for live routing from this principal-bound authorization.
    ///
    /// The route never grants access. Callers must still invoke [`Self::authorizes`] before
    /// delivery so policy and dependency generation changes fail closed.
    pub fn live_route(&self) -> Result<LiveAuthorizationRoute, String> {
        if self.policies.bypasses_rls() {
            return Ok(LiveAuthorizationRoute::Broadcast);
        }
        if self.policies.is_default_deny() {
            return Ok(LiveAuthorizationRoute::Deny);
        }

        let mut keys = HashSet::new();
        for policy in self.policies.policies() {
            match &policy.program {
                PolicyProgram::AuthorizationRelation(relation) => {
                    let [protected_column_id] = relation.protected_keys.as_slice() else {
                        return Err(
                            "live RLS membership routing requires one protected key".to_string()
                        );
                    };
                    let set = self.sets.get(&policy.policy_id).ok_or_else(|| {
                        format!("live RLS authorization set is missing for {}", policy.policy_id)
                    })?;
                    let values = set.single_keys().ok_or_else(|| {
                        "live RLS membership routing does not support composite keys".to_string()
                    })?;
                    keys.extend(values.into_iter().map(|value| (*protected_column_id, value)));
                },
                PolicyProgram::RowLocal {
                    expr:
                        BoundExprShape::ColumnEqualsPrincipal {
                            column_id,
                            principal: PrincipalExpr::CurrentUser,
                        },
                } => {
                    let column = self
                        .table
                        .columns
                        .iter()
                        .find(|column| column.column_id == *column_id)
                        .ok_or_else(|| {
                            format!("live RLS route column {column_id} does not exist")
                        })?;
                    let data_type =
                        column.data_type.to_arrow_type().map_err(|error| error.to_string())?;
                    let value =
                        parse_string_as_scalar(self.policies.principal().as_str(), &data_type)?;
                    keys.insert((*column_id, value));
                },
                PolicyProgram::RowLocal {
                    expr: BoundExprShape::ColumnEqualsScalar { column_id, value },
                } => {
                    let Some(value) = policy_scalar_text(value) else {
                        continue;
                    };
                    let column = self
                        .table
                        .columns
                        .iter()
                        .find(|column| column.column_id == *column_id)
                        .ok_or_else(|| {
                            format!("live RLS route column {column_id} does not exist")
                        })?;
                    let data_type =
                        column.data_type.to_arrow_type().map_err(|error| error.to_string())?;
                    let value = parse_string_as_scalar(&value, &data_type)?;
                    keys.insert((*column_id, value));
                },
                PolicyProgram::RowLocal {
                    expr: BoundExprShape::Literal(true),
                } => return Ok(LiveAuthorizationRoute::Broadcast),
                PolicyProgram::RowLocal {
                    expr: BoundExprShape::Literal(false),
                } => {},
                PolicyProgram::RowLocal { .. } => {
                    return Err("live RLS policy is not keyed by a bounded equality".to_string())
                },
            }
        }

        if keys.is_empty() {
            Ok(LiveAuthorizationRoute::Deny)
        } else {
            Ok(LiveAuthorizationRoute::Keyed(keys))
        }
    }

    /// Evaluates a batch with two epoch checks instead of two checks per row.
    pub fn authorizes_all<'a>(&self, rows: impl IntoIterator<Item = &'a Row>) -> bool {
        self.is_current()
            && rows
                .into_iter()
                .all(|row| self.policies.authorizes_row_with_sets(&self.table, row, &self.sets))
            && self.is_current()
    }

    /// Filters one resolved MVCC batch and rejects the entire result if an epoch changes.
    pub fn filter_authorized<T>(
        &self,
        items: impl IntoIterator<Item = T>,
        row: impl Fn(&T) -> &Row,
    ) -> Option<Vec<T>> {
        if !self.is_current() {
            return None;
        }
        let authorized = items
            .into_iter()
            .filter(|item| {
                self.policies.authorizes_row_with_sets(&self.table, row(item), &self.sets)
            })
            .collect();
        self.is_current().then_some(authorized)
    }
}

fn policy_scalar_text(value: &PolicyScalar) -> Option<String> {
    match value {
        PolicyScalar::Null => None,
        PolicyScalar::Boolean(value) => Some(value.to_string()),
        PolicyScalar::Int64(value) => Some(value.to_string()),
        PolicyScalar::UInt64(value) => Some(value.to_string()),
        PolicyScalar::Float64(value) => Some(value.to_string()),
        PolicyScalar::String(value) => Some(value.clone()),
    }
}
