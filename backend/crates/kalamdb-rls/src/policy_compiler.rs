use std::sync::Arc;

use kalamdb_commons::{
    schemas::TableDefinition, AuthorizationRelation, BoundExprShape, InvalidationStrategy,
    PolicyProgram, PolicyScalar, PredicateOperator, PrincipalExpr, ScalarPredicate, TableId,
};
use kalamdb_sql::parser::utils::parse_sql_expression;
use sqlparser::{
    ast::{
        BinaryOperator, Expr, FunctionArguments, GroupByExpr, ObjectName, Query, Select,
        SelectItem, SetExpr, TableFactor, UnaryOperator, Value,
    },
    dialect::PostgreSqlDialect,
};

/// Resolves table definitions while compiling a policy.
pub trait PolicyTableResolver {
    fn resolve_table(&self, table_id: &TableId) -> Result<Arc<TableDefinition>, String>;
}

/// Compiles policy SQL into user-independent authorization semantics.
pub struct PolicyCompiler<R> {
    resolver: R,
}

impl<R> PolicyCompiler<R>
where
    R: PolicyTableResolver,
{
    pub fn new(resolver: R) -> Self {
        Self { resolver }
    }

    pub fn compile(
        &self,
        protected_table: &TableDefinition,
        expression_sql: &str,
    ) -> Result<PolicyProgram, String> {
        let dialect = PostgreSqlDialect {};
        let expression = parse_sql_expression(expression_sql, &dialect)
            .map_err(|error| format!("invalid policy expression: {error}"))?;

        match expression {
            Expr::InSubquery {
                expr,
                subquery,
                negated: false,
            } => self.compile_in_subquery(protected_table, &expr, &subquery),
            Expr::Exists {
                subquery,
                negated: false,
            } => self.compile_exists(protected_table, &subquery),
            Expr::InSubquery { .. } | Expr::Exists { .. } => {
                Err("negated authorization relations are not supported".to_string())
            },
            expression => self
                .compile_row_local(protected_table, &expression)
                .map(|expr| PolicyProgram::RowLocal { expr }),
        }
    }

    fn compile_in_subquery(
        &self,
        protected_table: &TableDefinition,
        protected_expr: &Expr,
        query: &Query,
    ) -> Result<PolicyProgram, String> {
        let protected_column = column_name(protected_expr)
            .ok_or_else(|| "membership key must be a protected-table column".to_string())?;
        let protected_column_id = resolve_column_id(protected_table, protected_column.1)?;
        let (select, relation_table_id, relation_qualifier) =
            self.authorization_select(protected_table, query)?;

        let projected = match select.projection.as_slice() {
            [SelectItem::UnnamedExpr(expression)] => column_name(expression)
                .ok_or_else(|| "authorization relation must project one column".to_string())?,
            _ => return Err("authorization relation must project exactly one column".to_string()),
        };
        ensure_relation_column(projected.0, &relation_qualifier)?;

        let relation_table = self.resolver.resolve_table(&relation_table_id)?;
        ensure_shared_relation(&relation_table)?;
        let relation_key_id = resolve_column_id(&relation_table, projected.1)?;
        let selection = select
            .selection
            .as_ref()
            .ok_or_else(|| "authorization relation must restrict CURRENT_USER".to_string())?;
        let (principal_column, static_predicates) =
            compile_relation_predicates(selection, &relation_table, &relation_qualifier, false)?;

        Ok(PolicyProgram::AuthorizationRelation(AuthorizationRelation {
            protected_table: table_id(protected_table),
            protected_keys: vec![protected_column_id],
            relation_table: relation_table_id.clone(),
            relation_keys: vec![relation_key_id],
            principal_column,
            principal: PrincipalExpr::CurrentUser,
            static_predicates,
            dependencies: vec![relation_table_id],
            invalidation: InvalidationStrategy::TargetedPrincipal,
        }))
    }

    fn compile_exists(
        &self,
        protected_table: &TableDefinition,
        query: &Query,
    ) -> Result<PolicyProgram, String> {
        let (select, relation_table_id, relation_qualifier) =
            self.authorization_select(protected_table, query)?;
        if select.projection.len() != 1 {
            return Err("EXISTS authorization relation must have one projection".to_string());
        }
        let relation_table = self.resolver.resolve_table(&relation_table_id)?;
        ensure_shared_relation(&relation_table)?;
        let selection = select
            .selection
            .as_ref()
            .ok_or_else(|| "authorization relation must correlate a key".to_string())?;

        let predicates = flatten_and(selection);
        let mut principal_column = None;
        let mut key_pair = None;
        let mut static_predicates = Vec::new();
        for predicate in predicates {
            if let Some(column) = principal_equality(predicate, &relation_qualifier) {
                principal_column = Some(resolve_column_id(&relation_table, column)?);
                continue;
            }
            if let Some((relation_column, protected_column)) = correlation_equality(
                predicate,
                &relation_qualifier,
                protected_table.table_name.as_str(),
            ) {
                key_pair = Some((
                    resolve_column_id(&relation_table, relation_column)?,
                    resolve_column_id(protected_table, protected_column)?,
                ));
                continue;
            }
            static_predicates.push(compile_static_predicate(
                predicate,
                &relation_table,
                &relation_qualifier,
            )?);
        }

        let principal_column = principal_column
            .ok_or_else(|| "authorization relation must restrict CURRENT_USER".to_string())?;
        let (relation_key, protected_key) = key_pair
            .ok_or_else(|| "EXISTS authorization relation must correlate one key".to_string())?;

        Ok(PolicyProgram::AuthorizationRelation(AuthorizationRelation {
            protected_table: table_id(protected_table),
            protected_keys: vec![protected_key],
            relation_table: relation_table_id.clone(),
            relation_keys: vec![relation_key],
            principal_column,
            principal: PrincipalExpr::CurrentUser,
            static_predicates,
            dependencies: vec![relation_table_id],
            invalidation: InvalidationStrategy::TargetedPrincipal,
        }))
    }

    fn authorization_select<'a>(
        &self,
        protected_table: &TableDefinition,
        query: &'a Query,
    ) -> Result<(&'a Select, TableId, String), String> {
        if query.with.is_some()
            || query.order_by.is_some()
            || query.limit_clause.is_some()
            || query.fetch.is_some()
            || !query.locks.is_empty()
            || query.for_clause.is_some()
        {
            return Err(
                "authorization relation cannot use CTE, ordering, limits, or locks".to_string()
            );
        }
        let SetExpr::Select(select) = query.body.as_ref() else {
            return Err("authorization relation must be a single SELECT".to_string());
        };
        if select.distinct.is_some()
            || select.from.len() != 1
            || !select.from[0].joins.is_empty()
            || !group_by_is_empty(&select.group_by)
            || select.having.is_some()
            || !select.named_window.is_empty()
            || select.qualify.is_some()
        {
            return Err("authorization relation cannot use joins, grouping, windows, or DISTINCT"
                .to_string());
        }

        let TableFactor::Table {
            name,
            alias,
            args: None,
            ..
        } = &select.from[0].relation
        else {
            return Err("authorization relation must read one table".to_string());
        };
        let relation_table_id = object_name_to_table_id(name, &protected_table.namespace_id)?;
        if relation_table_id == table_id(protected_table) {
            return Err("RLS policy cannot reference its protected table".to_string());
        }
        let qualifier = alias
            .as_ref()
            .map(|alias| alias.name.value.clone())
            .unwrap_or_else(|| relation_table_id.table_name().as_str().to_string());
        Ok((select, relation_table_id, qualifier))
    }

    fn compile_row_local(
        &self,
        protected_table: &TableDefinition,
        expression: &Expr,
    ) -> Result<BoundExprShape, String> {
        match expression {
            Expr::Nested(expression) => self.compile_row_local(protected_table, expression),
            Expr::Value(value) => match &value.value {
                Value::Boolean(value) => Ok(BoundExprShape::Literal(*value)),
                _ => Err("row-local policy must evaluate to boolean".to_string()),
            },
            Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr,
            } => Ok(BoundExprShape::Not(Box::new(self.compile_row_local(protected_table, expr)?))),
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } => Ok(BoundExprShape::And(vec![
                self.compile_row_local(protected_table, left)?,
                self.compile_row_local(protected_table, right)?,
            ])),
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Or,
                right,
            } => Ok(BoundExprShape::Or(vec![
                self.compile_row_local(protected_table, left)?,
                self.compile_row_local(protected_table, right)?,
            ])),
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq,
                right,
            } => compile_row_equality(protected_table, left, right),
            _ => {
                Err("policy expression is not a supported row-local or membership shape"
                    .to_string())
            },
        }
    }

    pub fn covering_membership_index_warning(&self, program: &PolicyProgram) -> Option<String> {
        let PolicyProgram::AuthorizationRelation(relation) = program else {
            return None;
        };
        let relation_table = self.resolver.resolve_table(&relation.relation_table).ok()?;
        if has_covering_membership_primary_key(&relation_table, relation) {
            None
        } else {
            Some(format!(
                "warning: relation {} has no covering primary key on (principal, relation_key); \
                 PointGuard falls back to a membership scan or cache",
                relation.relation_table
            ))
        }
    }
}

fn table_id(table: &TableDefinition) -> TableId {
    TableId::new(table.namespace_id.clone(), table.table_name.clone())
}

fn has_covering_membership_primary_key(
    table: &TableDefinition,
    relation: &AuthorizationRelation,
) -> bool {
    let mut pk_columns =
        table.columns.iter().filter(|column| column.is_primary_key).collect::<Vec<_>>();
    pk_columns.sort_by_key(|column| column.ordinal_position);
    let pk_ids = pk_columns.into_iter().map(|column| column.column_id).collect::<Vec<_>>();
    let mut required = vec![relation.principal_column];
    required.extend_from_slice(&relation.relation_keys);
    pk_ids == required
}

fn ensure_shared_relation(table: &TableDefinition) -> Result<(), String> {
    if table.table_type == kalamdb_commons::TableType::Shared {
        Ok(())
    } else {
        Err(format!("authorization relation {} must be a shared table", table.table_name))
    }
}

fn resolve_column_id(table: &TableDefinition, column_name: &str) -> Result<u64, String> {
    table
        .columns
        .iter()
        .find(|column| column.column_name.eq_ignore_ascii_case(column_name))
        .map(|column| column.column_id)
        .ok_or_else(|| format!("column '{}' does not exist on {}", column_name, table.table_name))
}

fn object_name_to_table_id(
    name: &ObjectName,
    default_namespace: &kalamdb_commons::NamespaceId,
) -> Result<TableId, String> {
    let parts = name
        .0
        .iter()
        .map(|part| {
            part.as_ident()
                .map(|ident| ident.value.as_str())
                .ok_or_else(|| "authorization table must contain identifiers".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    match parts.as_slice() {
        [table] => TableId::try_from_strings(default_namespace.as_str(), table),
        [namespace, table] => TableId::try_from_strings(namespace, table),
        _ => Err("authorization table must be <table> or <namespace>.<table>".to_string()),
    }
}

fn column_name(expression: &Expr) -> Option<(Option<&str>, &str)> {
    match expression {
        Expr::Identifier(identifier) => Some((None, identifier.value.as_str())),
        Expr::CompoundIdentifier(identifiers) if identifiers.len() == 2 => {
            Some((Some(identifiers[0].value.as_str()), identifiers[1].value.as_str()))
        },
        Expr::Nested(expression) => column_name(expression),
        _ => None,
    }
}

fn ensure_relation_column(qualifier: Option<&str>, expected: &str) -> Result<(), String> {
    if qualifier.is_none_or(|qualifier| qualifier.eq_ignore_ascii_case(expected)) {
        Ok(())
    } else {
        Err("authorization projection must come from its relation".to_string())
    }
}

fn is_current_user(expression: &Expr) -> bool {
    match expression {
        Expr::Identifier(identifier) => identifier.value.eq_ignore_ascii_case("CURRENT_USER"),
        Expr::Function(function) => {
            function.name.to_string().eq_ignore_ascii_case("CURRENT_USER")
                && (matches!(function.args, FunctionArguments::None)
                    || matches!(&function.args, FunctionArguments::List(arguments) if arguments.args.is_empty()))
        },
        _ => false,
    }
}

fn flatten_and(expression: &Expr) -> Vec<&Expr> {
    match expression {
        Expr::Nested(expression) => flatten_and(expression),
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let mut predicates = flatten_and(left);
            predicates.extend(flatten_and(right));
            predicates
        },
        _ => vec![expression],
    }
}

fn principal_equality<'a>(expression: &'a Expr, relation_qualifier: &str) -> Option<&'a str> {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Eq,
        right,
    } = expression
    else {
        return None;
    };
    for (column, principal) in [
        (left.as_ref(), right.as_ref()),
        (right.as_ref(), left.as_ref()),
    ] {
        let Some((qualifier, column_name)) = column_name(column) else {
            continue;
        };
        if qualifier.is_none_or(|value| value.eq_ignore_ascii_case(relation_qualifier))
            && is_current_user(principal)
        {
            return Some(column_name);
        }
    }
    None
}

fn correlation_equality<'a>(
    expression: &'a Expr,
    relation_qualifier: &str,
    protected_qualifier: &str,
) -> Option<(&'a str, &'a str)> {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Eq,
        right,
    } = expression
    else {
        return None;
    };
    for (relation, protected) in [
        (left.as_ref(), right.as_ref()),
        (right.as_ref(), left.as_ref()),
    ] {
        let Some((Some(relation_owner), relation_column)) = column_name(relation) else {
            continue;
        };
        let Some((Some(protected_owner), protected_column)) = column_name(protected) else {
            continue;
        };
        if relation_owner.eq_ignore_ascii_case(relation_qualifier)
            && protected_owner.eq_ignore_ascii_case(protected_qualifier)
        {
            return Some((relation_column, protected_column));
        }
    }
    None
}

fn compile_relation_predicates(
    selection: &Expr,
    relation_table: &TableDefinition,
    relation_qualifier: &str,
    allow_correlation: bool,
) -> Result<(u64, Vec<ScalarPredicate>), String> {
    let mut principal_column = None;
    let mut static_predicates = Vec::new();
    for predicate in flatten_and(selection) {
        if let Some(column) = principal_equality(predicate, relation_qualifier) {
            if principal_column.is_some() {
                return Err("authorization relation has multiple principal predicates".to_string());
            }
            principal_column = Some(resolve_column_id(relation_table, column)?);
        } else if allow_correlation {
            return Err("unexpected correlation predicate".to_string());
        } else {
            static_predicates.push(compile_static_predicate(
                predicate,
                relation_table,
                relation_qualifier,
            )?);
        }
    }
    principal_column
        .map(|column| (column, static_predicates))
        .ok_or_else(|| "authorization relation must restrict CURRENT_USER".to_string())
}

fn compile_static_predicate(
    expression: &Expr,
    relation_table: &TableDefinition,
    relation_qualifier: &str,
) -> Result<ScalarPredicate, String> {
    let Expr::BinaryOp { left, op, right } = expression else {
        return Err("authorization relation predicate must be scalar equality".to_string());
    };
    let operator = match op {
        BinaryOperator::Eq => PredicateOperator::Eq,
        BinaryOperator::NotEq => PredicateOperator::NotEq,
        _ => return Err("authorization relation predicate operator is not supported".to_string()),
    };
    for (column, literal) in [
        (left.as_ref(), right.as_ref()),
        (right.as_ref(), left.as_ref()),
    ] {
        let Some((qualifier, column_name)) = column_name(column) else {
            continue;
        };
        ensure_relation_column(qualifier, relation_qualifier)?;
        if let Some(value) = policy_scalar(literal) {
            return Ok(ScalarPredicate {
                column_id: resolve_column_id(relation_table, column_name)?,
                operator,
                value,
            });
        }
    }
    Err("authorization relation predicate must compare a relation column to a literal".to_string())
}

fn compile_row_equality(
    protected_table: &TableDefinition,
    left: &Expr,
    right: &Expr,
) -> Result<BoundExprShape, String> {
    for (column, value) in [(left, right), (right, left)] {
        let Some((qualifier, column_name)) = column_name(column) else {
            continue;
        };
        if qualifier.is_some_and(|qualifier| {
            !qualifier.eq_ignore_ascii_case(protected_table.table_name.as_str())
        }) {
            return Err("row-local policy cannot reference another table".to_string());
        }
        let column_id = resolve_column_id(protected_table, column_name)?;
        if is_current_user(value) {
            return Ok(BoundExprShape::ColumnEqualsPrincipal {
                column_id,
                principal: PrincipalExpr::CurrentUser,
            });
        }
        if let Some(value) = policy_scalar(value) {
            return Ok(BoundExprShape::ColumnEqualsScalar { column_id, value });
        }
    }
    Err(
        "row-local equality must compare a protected column to CURRENT_USER or a literal"
            .to_string(),
    )
}

fn policy_scalar(expression: &Expr) -> Option<PolicyScalar> {
    let Expr::Value(value) = expression else {
        return None;
    };
    match &value.value {
        Value::Boolean(value) => Some(PolicyScalar::Boolean(*value)),
        Value::Null => Some(PolicyScalar::Null),
        Value::SingleQuotedString(value) | Value::DoubleQuotedString(value) => {
            Some(PolicyScalar::String(value.clone()))
        },
        Value::Number(value, _) => value
            .parse::<i64>()
            .map(PolicyScalar::Int64)
            .or_else(|_| value.parse::<u64>().map(PolicyScalar::UInt64))
            .or_else(|_| value.parse::<f64>().map(PolicyScalar::Float64))
            .ok(),
        _ => None,
    }
}

fn group_by_is_empty(group_by: &GroupByExpr) -> bool {
    matches!(group_by, GroupByExpr::Expressions(expressions, modifiers) if expressions.is_empty() && modifiers.is_empty())
}
