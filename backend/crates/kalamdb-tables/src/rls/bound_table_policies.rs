use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{
    models::rows::Row, schemas::TableDefinition, BoundExprShape, PolicyCommand, PolicyId,
    PolicyProgram, PolicyScalar, PolicyTarget, PrincipalExpr, Role, TablePolicy, UserId,
};
use datafusion::scalar::ScalarValue;

use super::AuthorizationSet;

#[derive(Debug, Clone)]
pub struct BoundPolicy {
    pub policy_id: PolicyId,
    pub policy_generation: u64,
    pub program: PolicyProgram,
}

#[derive(Debug, Clone)]
pub struct BoundTablePolicies {
    principal: UserId,
    bypass: bool,
    policies: Vec<BoundPolicy>,
}

impl BoundTablePolicies {
    pub fn bind(
        policies: Vec<TablePolicy>,
        principal: UserId,
        role: Role,
        command: PolicyCommand) -> Self {
        Self::bind_with_program(policies, principal, role, command, false)
    }

    pub fn bind_check(
        policies: Vec<TablePolicy>,
        principal: UserId,
        role: Role,
        command: PolicyCommand) -> Self {
        Self::bind_with_program(policies, principal, role, command, true)
    }

    fn bind_with_program(
        policies: Vec<TablePolicy>,
        principal: UserId,
        role: Role,
        command: PolicyCommand,
        check: bool) -> Self {
        let bypass = matches!(role, Role::System | Role::Dba);
        let policies = if bypass {
            Vec::new()
        } else {
            policies
                .into_iter()
                .filter(|policy| policy.targets.iter().any(|target| target_applies(target, role)))
                .filter_map(|policy| {
                    let program = if check {
                        policy.check_program_for(command)
                    } else {
                        policy.using_program_for(command)
                    };
                    program.cloned().map(|program| BoundPolicy {
                        policy_id: policy.policy_id,
                        policy_generation: policy.policy_generation,
                        program,
                    })
                })
                .collect()
        };
        Self {
            principal,
            bypass,
            policies,
        }
    }

    pub fn bypasses_rls(&self) -> bool {
        self.bypass
    }

    pub fn is_default_deny(&self) -> bool {
        !self.bypass && self.policies.is_empty()
    }

    pub fn policies(&self) -> &[BoundPolicy] {
        &self.policies
    }

    pub fn single_membership_policy(
        &self) -> Option<(&BoundPolicy, &kalamdb_commons::AuthorizationRelation)> {
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
                PolicyProgram::RowLocal { expr } => collect_row_local_column_ids(expr, &mut column_ids),
                PolicyProgram::AuthorizationRelation(relation) => {
                    column_ids.extend(relation.protected_keys.iter().copied());
                },
            }
        }
        column_ids.sort_unstable();
        column_ids.dedup();
        column_ids
    }

    pub fn authorizes_row(&self, table: &TableDefinition, row: &Row) -> bool {
        self.authorizes_row_with_sets(table, row, &HashMap::new())
    }

    pub fn authorizes_row_with_sets(
        &self,
        table: &TableDefinition,
        row: &Row,
        authorization_sets: &HashMap<PolicyId, Arc<AuthorizationSet>>) -> bool {
        if self.bypass {
            return true;
        }
        self.policies.iter().any(|policy| match &policy.program {
            PolicyProgram::RowLocal { expr } => {
                evaluate_row_local(expr, table, row, &self.principal)
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

fn target_applies(target: &PolicyTarget, role: Role) -> bool {
    matches!(target, PolicyTarget::Public)
        || matches!(target, PolicyTarget::Role(target_role) if *target_role == role)
}

fn evaluate_row_local(
    expression: &BoundExprShape,
    table: &TableDefinition,
    row: &Row,
    principal: &UserId) -> bool {
    match expression {
        BoundExprShape::Literal(value) => *value,
        BoundExprShape::ColumnEqualsPrincipal { column_id, principal: principal_expr } => {
            if !matches!(principal_expr, PrincipalExpr::CurrentUser) {
                return false;
            }
            column_value(table, row, *column_id)
                .is_some_and(|value| scalar_equals_principal(value, principal))
        },
        BoundExprShape::ColumnEqualsScalar { column_id, value } => column_value(table, row, *column_id)
            .is_some_and(|row_value| scalar_equals_policy(row_value, value)),
        BoundExprShape::And(expressions) => expressions
            .iter()
            .all(|expression| evaluate_row_local(expression, table, row, principal)),
        BoundExprShape::Or(expressions) => expressions
            .iter()
            .any(|expression| evaluate_row_local(expression, table, row, principal)),
        BoundExprShape::Not(expression) => !evaluate_row_local(expression, table, row, principal),
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
        | (ScalarValue::LargeUtf8(Some(value)), PolicyScalar::String(expected)) => value == expected,
        _ => false,
    }
}
