//! Immutable named-query plans from the canonical SQL AST.
//!
//! Analysis does not execute SQL. Cardinality, parameters, keys, and live
//! eligibility come from the parsed statement plus optional catalog metadata.

use std::{collections::BTreeSet, fmt, ops::ControlFlow};

use sqlparser::ast::{
    Expr, Function, GroupByExpr, Ident, LimitClause, ObjectName, ObjectNamePart, OrderByExpr,
    OrderByKind, Query, Select, SelectItem, SetExpr, Statement, TableFactor, TableWithJoins, Value,
    Visit, Visitor,
};

use super::snapshot::{ContractField, ContractSnapshot, ContractTable};
use crate::{
    parser::QueryParser,
    query_features::{supports_general_query_feature, GeneralQueryFeature},
    KalamParser, CURRENT_CONTRACT_EPOCH,
};

/// Failure while analyzing a named query or checking cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPlanError {
    pub message: String,
}

impl QueryPlanError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn cancelled() -> Self {
        Self::new("query cancelled before execution")
    }
}

impl fmt::Display for QueryPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QueryPlanError {}

/// How many rows a named query is defined to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryCardinality {
    /// Zero or more rows.
    Many,
    /// Zero or one row, from a literal `LIMIT 1`.
    AtMostOne,
    /// Exactly one row, from an aggregate without `GROUP BY`.
    Scalar,
}

impl QueryCardinality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Many => "many",
            Self::AtMostOne => "at_most_one",
            Self::Scalar => "scalar",
        }
    }
}

/// Where a predicate operand came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicateOperand {
    Parameter { ordinal: u32 },
    Literal,
    Null,
    Expression,
}

/// One comparison extracted from the AST. Boolean connectives stay structure,
/// not extra predicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPredicate {
    pub column:   String,
    pub operator: String,
    pub operand:  PredicateOperand,
}

/// Positional parameter discovered in the statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParameter {
    pub ordinal:         u32,
    pub placeholder:     String,
    /// Set when every comparison of this placeholder targets one column.
    pub compared_column: Option<String>,
}

/// Output column. Expression aliases do not become keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryColumn {
    pub name:          String,
    pub source_column: Option<String>,
    pub type_name:     Option<String>,
    pub not_null:      bool,
    pub is_read_only:  bool,
    pub key_ordinal:   Option<u16>,
    pub expression:    bool,
}

/// Sort key taken from `ORDER BY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryOrderKey {
    pub expression:  String,
    pub ascending:   bool,
    pub nulls_first: Option<bool>,
}

/// One-shot pagination. Live windows are a separate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueryPagination {
    pub limit:  Option<u64>,
    pub offset: Option<u64>,
}

/// Features observed in the AST. Unsupported features fail analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueryFeatures {
    pub joins:         bool,
    pub subqueries:    bool,
    pub grouping:      bool,
    pub windows:       bool,
    pub distinct:      bool,
    pub aggregates:    bool,
    pub pagination:    bool,
    pub cte:           bool,
    pub set_operation: bool,
    pub locking:       bool,
}

/// Whether this definition may be subscribed with the current live shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEligibility {
    pub eligible: bool,
    pub reason:   Option<String>,
}

/// Single relation referenced by the query, when there is exactly one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuerySourceTable {
    pub schema:   Option<String>,
    pub name:     String,
    pub table_id: Option<String>,
}

/// Cancellation is caller-controlled. The plan itself does no I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryCancellation {
    pub check_before_execute: bool,
}

impl QueryCancellation {
    pub const REQUIRED: Self = Self {
        check_before_execute: true,
    };

    pub fn check(self, cancelled: bool) -> Result<(), QueryPlanError> {
        if self.check_before_execute && cancelled {
            Err(QueryPlanError::cancelled())
        } else {
            Ok(())
        }
    }
}

/// Immutable plan for one named `SELECT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedQueryPlan {
    pub epoch:                u16,
    pub name:                 String,
    pub sql:                  String,
    pub table:                Option<QuerySourceTable>,
    pub parameters:           Vec<QueryParameter>,
    pub columns:              Vec<QueryColumn>,
    pub projects_all_columns: bool,
    pub predicates:           Vec<QueryPredicate>,
    pub order_by:             Vec<QueryOrderKey>,
    pub pagination:           QueryPagination,
    pub cardinality:          QueryCardinality,
    pub key_columns:          Vec<String>,
    pub key_complete:         bool,
    pub live:                 LiveEligibility,
    pub features:             QueryFeatures,
    pub cancellation:         QueryCancellation,
}

impl TypedQueryPlan {
    pub fn check_cancellation(&self, cancelled: bool) -> Result<(), QueryPlanError> {
        self.cancellation.check(cancelled)
    }
}

/// Analyze one named query. `catalog` binds types and real keys when present.
pub fn analyze_named_query(
    name: &str,
    sql: &str,
    catalog: Option<&ContractSnapshot>,
) -> Result<TypedQueryPlan, QueryPlanError> {
    if name.trim().is_empty() {
        return Err(QueryPlanError::new("named query requires a name"));
    }
    let statements = KalamParser::parse(sql).map_err(map_parse_error)?;
    if statements.len() != 1 {
        return Err(QueryPlanError::new("named query must be a single SELECT statement"));
    }
    let Statement::Query(query) = &statements[0] else {
        return Err(QueryPlanError::new("named query must be a single SELECT statement"));
    };

    let mut features = QueryFeatures {
        cte: query.with.is_some(),
        locking: !query.locks.is_empty() || query.for_clause.is_some(),
        ..QueryFeatures::default()
    };
    let pagination = pagination_from_query(query, &mut features)?;
    let order_by = order_by_from_query(query);

    let mut walker = ExprWalker::default();
    let mut tables = Vec::new();
    match query.body.as_ref() {
        SetExpr::Select(select) => {
            features.distinct = select.distinct.is_some();
            features.grouping = has_group_by(&select.group_by) || select.having.is_some();
            collect_tables(select, &mut features, &mut tables);
            walk_select(select, &mut walker);
        },
        SetExpr::SetOperation { .. } => features.set_operation = true,
        _ => {
            return Err(QueryPlanError::new("named query must be a single SELECT statement"));
        },
    }
    walk_order_and_limit(query, &mut walker);
    if walker.lambda && !supports_general_query_feature(GeneralQueryFeature::LambdaArrayFunctions) {
        return Err(QueryPlanError::new(
            "lambda array functions are not supported in named queries",
        ));
    }
    features.subqueries |= walker.subqueries;
    features.windows = walker.windows;
    features.aggregates = walker.aggregates;

    let source = match tables.as_slice() {
        [parts] if !features.joins && !features.set_operation => Some(parts.clone()),
        _ => None,
    };
    let bound = match (catalog, source.as_ref()) {
        (Some(catalog), Some(parts)) => Some(resolve_table(catalog, parts)?),
        (Some(_), None) if !tables.is_empty() && features.joins => None,
        (Some(catalog), None) if tables.len() == 1 && !features.set_operation => {
            Some(resolve_table(catalog, &tables[0])?)
        },
        _ => None,
    };

    let (columns, projects_all_columns) = match query.body.as_ref() {
        SetExpr::Select(select) => {
            project_columns(select, bound.as_ref().map(|binding| binding.table))?
        },
        _ => (Vec::new(), false),
    };

    let (key_columns, key_complete) =
        key_projection(bound.as_ref().map(|binding| binding.table), &columns);
    let cardinality = cardinality_for(&features, pagination);
    let live = live_eligibility(sql);
    let parameters = finish_parameters(&walker.parameters);

    Ok(TypedQueryPlan {
        epoch: CURRENT_CONTRACT_EPOCH,
        name: name.to_string(),
        sql: sql.to_string(),
        table: bound
            .map(|binding| binding.source)
            .or_else(|| source.as_ref().map(|parts| unbound_source(parts))),
        parameters,
        columns,
        projects_all_columns,
        predicates: walker.predicates,
        order_by,
        pagination,
        cardinality,
        key_columns,
        key_complete,
        live,
        features,
        cancellation: QueryCancellation::REQUIRED,
    })
}

struct BoundTable<'a> {
    table:  &'a ContractTable,
    source: QuerySourceTable,
}

fn resolve_table<'a>(
    catalog: &'a ContractSnapshot,
    parts: &[String],
) -> Result<BoundTable<'a>, QueryPlanError> {
    let table = match parts {
        [name] => {
            let mut matches = catalog
                .tables
                .values()
                .filter(|table| table.name.eq_ignore_ascii_case(name))
                .collect::<Vec<_>>();
            match matches.len() {
                0 => {
                    return Err(QueryPlanError::new(format!("unknown table {name}")));
                },
                1 => matches.remove(0),
                _ => {
                    return Err(QueryPlanError::new(format!("ambiguous table {name}")));
                },
            }
        },
        [schema, name] => {
            let table_id = format!("{}.{}", schema.to_ascii_lowercase(), name.to_ascii_lowercase());
            catalog
                .tables
                .get(&table_id)
                .ok_or_else(|| QueryPlanError::new(format!("unknown table {schema}.{name}")))?
        },
        _ => {
            return Err(QueryPlanError::new("named query table reference must be [schema.]table"));
        },
    };
    Ok(BoundTable {
        source: QuerySourceTable {
            schema:   Some(table.schema.clone()),
            name:     table.name.clone(),
            table_id: Some(table.table_id.clone()),
        },
        table,
    })
}

fn unbound_source(parts: &[String]) -> QuerySourceTable {
    match parts {
        [name] => QuerySourceTable {
            schema:   None,
            name:     name.clone(),
            table_id: None,
        },
        [schema, name] => QuerySourceTable {
            schema:   Some(schema.clone()),
            name:     name.clone(),
            table_id: None,
        },
        _ => QuerySourceTable {
            schema:   None,
            name:     parts.join("."),
            table_id: None,
        },
    }
}

fn project_columns(
    select: &Select,
    table: Option<&ContractTable>,
) -> Result<(Vec<QueryColumn>, bool), QueryPlanError> {
    let mut wildcard = false;
    let mut named = Vec::new();
    for item in &select.projection {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => wildcard = true,
            SelectItem::UnnamedExpr(expr) => named.push(projected_expr(expr, None)?),
            SelectItem::ExprWithAlias { expr, alias } => {
                named.push(projected_expr(expr, Some(alias))?)
            },
            _ => {
                return Err(QueryPlanError::new("unsupported SELECT projection"));
            },
        }
    }
    if wildcard && !named.is_empty() {
        return Err(QueryPlanError::new("named query projection mixes * with columns"));
    }
    if wildcard {
        let columns = match table {
            Some(table) => table
                .fields
                .iter()
                .filter(|field| !field.dropped)
                .map(|field| column_from_field(field, false))
                .collect(),
            None => Vec::new(),
        };
        return Ok((columns, true));
    }

    if let Some(table) = table {
        for column in &mut named {
            if column.expression {
                continue;
            }
            let Some(source) = column.source_column.as_deref() else {
                continue;
            };
            let Some(field) = table
                .fields
                .iter()
                .find(|field| !field.dropped && field.name.eq_ignore_ascii_case(source))
            else {
                return Err(QueryPlanError::new(format!(
                    "column {source} is not in {}.{}",
                    table.schema, table.name
                )));
            };
            column.type_name = Some(field.type_name.clone());
            column.not_null = field.not_null;
            column.is_read_only = field.is_read_only;
            column.key_ordinal = field.primary_key_ordinal;
        }
    }
    Ok((named, false))
}

fn projected_expr(expr: &Expr, alias: Option<&Ident>) -> Result<QueryColumn, QueryPlanError> {
    let source = column_name(expr);
    let expression = source.is_none();
    let name = alias
        .map(|ident| ident.value.clone())
        .or(source.clone())
        .unwrap_or_else(|| expr.to_string());
    Ok(QueryColumn {
        name,
        source_column: source,
        type_name: None,
        not_null: false,
        is_read_only: false,
        key_ordinal: None,
        expression,
    })
}

fn column_from_field(field: &ContractField, expression: bool) -> QueryColumn {
    QueryColumn {
        name: field.name.clone(),
        source_column: (!expression).then(|| field.name.clone()),
        type_name: Some(field.type_name.clone()),
        not_null: field.not_null,
        is_read_only: field.is_read_only,
        key_ordinal: field.primary_key_ordinal,
        expression,
    }
}

fn key_projection(table: Option<&ContractTable>, columns: &[QueryColumn]) -> (Vec<String>, bool) {
    let Some(table) = table else {
        return (Vec::new(), false);
    };
    let keys = table.ordered_key_fields();
    if keys.is_empty() {
        return (Vec::new(), false);
    }
    let complete = keys.iter().all(|key| {
        columns.iter().any(|column| {
            !column.expression
                && column
                    .source_column
                    .as_deref()
                    .is_some_and(|source| source.eq_ignore_ascii_case(&key.name))
        })
    });
    if !complete {
        return (Vec::new(), false);
    }
    (keys.into_iter().map(|field| field.name.clone()).collect(), true)
}

fn cardinality_for(features: &QueryFeatures, pagination: QueryPagination) -> QueryCardinality {
    if features.aggregates && !features.grouping && !features.distinct {
        QueryCardinality::Scalar
    } else if pagination.limit == Some(1) && pagination.offset.unwrap_or(0) == 0 {
        QueryCardinality::AtMostOne
    } else {
        QueryCardinality::Many
    }
}

fn live_eligibility(sql: &str) -> LiveEligibility {
    match QueryParser::analyze_subscription_query(sql) {
        Ok(_) => LiveEligibility {
            eligible: true,
            reason:   None,
        },
        Err(error) => LiveEligibility {
            eligible: false,
            reason:   Some(error.into_user_message()),
        },
    }
}

fn pagination_from_query(
    query: &Query,
    features: &mut QueryFeatures,
) -> Result<QueryPagination, QueryPlanError> {
    if query.fetch.is_some() {
        features.pagination = true;
    }
    let Some(clause) = &query.limit_clause else {
        return Ok(QueryPagination::default());
    };
    features.pagination = true;
    let (limit, offset) = match clause {
        LimitClause::LimitOffset {
            limit,
            offset,
            limit_by,
        } => {
            if !limit_by.is_empty() {
                return Err(QueryPlanError::new("LIMIT BY is not supported in named queries"));
            }
            (limit.as_ref(), offset.as_ref().map(|offset| &offset.value))
        },
        LimitClause::OffsetCommaLimit { offset, limit } => (Some(limit), Some(offset)),
    };
    Ok(QueryPagination {
        limit:  limit.and_then(literal_u64),
        offset: offset.and_then(literal_u64),
    })
}

fn literal_u64(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::Number(text, false) => text.parse().ok(),
            _ => None,
        },
        _ => None,
    }
}

fn order_by_from_query(query: &Query) -> Vec<QueryOrderKey> {
    let Some(order_by) = &query.order_by else {
        return Vec::new();
    };
    let OrderByKind::Expressions(exprs) = &order_by.kind else {
        return Vec::new();
    };
    exprs.iter().map(order_key).collect()
}

fn order_key(item: &OrderByExpr) -> QueryOrderKey {
    QueryOrderKey {
        expression:  item.expr.to_string(),
        ascending:   item.options.asc.unwrap_or(true),
        nulls_first: item.options.nulls_first,
    }
}

fn has_group_by(group_by: &GroupByExpr) -> bool {
    match group_by {
        GroupByExpr::All(_) => true,
        GroupByExpr::Expressions(expressions, _) => !expressions.is_empty(),
    }
}

fn collect_tables(select: &Select, features: &mut QueryFeatures, tables: &mut Vec<Vec<String>>) {
    if select.from.len() > 1 {
        features.joins = true;
    }
    for from in &select.from {
        if !from.joins.is_empty() {
            features.joins = true;
        }
        collect_factor(&from.relation, features, tables);
        for join in &from.joins {
            collect_factor(&join.relation, features, tables);
        }
    }
}

fn collect_factor(
    factor: &TableFactor,
    features: &mut QueryFeatures,
    tables: &mut Vec<Vec<String>>,
) {
    match factor {
        TableFactor::Table { name, .. } => tables.push(object_parts(name)),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            features.joins = true;
            collect_join_tree(table_with_joins, features, tables);
        },
        _ => features.subqueries = true,
    }
}

fn collect_join_tree(
    table: &TableWithJoins,
    features: &mut QueryFeatures,
    tables: &mut Vec<Vec<String>>,
) {
    if !table.joins.is_empty() {
        features.joins = true;
    }
    collect_factor(&table.relation, features, tables);
    for join in &table.joins {
        collect_factor(&join.relation, features, tables);
    }
}

fn object_parts(name: &ObjectName) -> Vec<String> {
    name.0
        .iter()
        .filter_map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.clone()),
            _ => None,
        })
        .collect()
}

fn walk_select(select: &Select, walker: &mut ExprWalker) {
    for item in &select.projection {
        match item {
            SelectItem::UnnamedExpr(expr) => {
                let _ = expr.visit(walker);
            },
            SelectItem::ExprWithAlias { expr, .. } => {
                let _ = expr.visit(walker);
            },
            _ => {},
        }
    }
    if let Some(selection) = &select.selection {
        let _ = selection.visit(walker);
    }
    if let Some(having) = &select.having {
        let _ = having.visit(walker);
    }
}

fn walk_order_and_limit(query: &Query, walker: &mut ExprWalker) {
    if let Some(order_by) = &query.order_by {
        if let OrderByKind::Expressions(exprs) = &order_by.kind {
            for item in exprs {
                let _ = item.expr.visit(walker);
            }
        }
    }
    if let Some(clause) = &query.limit_clause {
        match clause {
            LimitClause::LimitOffset { limit, offset, .. } => {
                if let Some(limit) = limit {
                    let _ = limit.visit(walker);
                }
                if let Some(offset) = offset {
                    let _ = offset.value.visit(walker);
                }
            },
            LimitClause::OffsetCommaLimit { offset, limit } => {
                let _ = offset.visit(walker);
                let _ = limit.visit(walker);
            },
        }
    }
}

#[derive(Default)]
struct ExprWalker {
    parameters: Vec<ParamDraft>,
    predicates: Vec<QueryPredicate>,
    lambda:     bool,
    subqueries: bool,
    windows:    bool,
    aggregates: bool,
}

struct ParamDraft {
    placeholder: String,
    ordinal:     u32,
    columns:     BTreeSet<String>,
}

impl Visitor for ExprWalker {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Subquery(_) | Expr::Exists { .. } | Expr::InSubquery { .. } => {
                self.subqueries = true;
            },
            Expr::BinaryOp { left, op, right } => {
                let rendered = op.to_string();
                if (rendered == "->" || rendered == "->>") && !json_key_operand(right) {
                    self.lambda = true;
                } else if let Some(operator) = comparison_operator(&rendered) {
                    self.record_comparison(left, operator, right);
                }
            },
            Expr::IsNull(inner) => self.record_null_test(inner, "IS NULL"),
            Expr::IsNotNull(inner) => self.record_null_test(inner, "IS NOT NULL"),
            Expr::Like { expr, negated, .. } => {
                let operator = if *negated { "NOT LIKE" } else { "LIKE" };
                if let Some(column) = column_name(expr) {
                    self.predicates.push(QueryPredicate {
                        column,
                        operator: operator.to_string(),
                        operand: PredicateOperand::Expression,
                    });
                }
            },
            Expr::Function(function) => self.note_function(function),
            Expr::Value(value) => {
                if let Value::Placeholder(placeholder) = &value.value {
                    self.note_placeholder(placeholder);
                }
            },
            _ => {},
        }
        ControlFlow::Continue(())
    }
}

impl ExprWalker {
    fn note_function(&mut self, function: &Function) {
        if function.over.is_some() {
            self.windows = true;
        }
        let name = function
            .name
            .0
            .last()
            .and_then(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
                _ => None,
            })
            .unwrap_or("");
        if is_aggregate_name(name) {
            self.aggregates = true;
        }
    }

    fn note_placeholder(&mut self, placeholder: &str) {
        if self.parameters.iter().any(|param| param.placeholder == placeholder) {
            return;
        }
        self.parameters.push(ParamDraft {
            ordinal:     positional_ordinal(placeholder).unwrap_or(0),
            placeholder: placeholder.to_string(),
            columns:     BTreeSet::new(),
        });
    }

    fn record_comparison(&mut self, left: &Expr, operator: &str, right: &Expr) {
        let (column, operand_expr) = if let Some(column) = column_name(left) {
            (column, right)
        } else if let Some(column) = column_name(right) {
            (column, left)
        } else {
            return;
        };
        let operand = self.operand(operand_expr);
        if let PredicateOperand::Parameter { ordinal } = &operand {
            if let Some(param) = self.parameters.iter_mut().find(|param| param.ordinal == *ordinal)
            {
                param.columns.insert(column.clone());
            }
        }
        self.predicates.push(QueryPredicate {
            column,
            operator: operator.to_string(),
            operand,
        });
    }

    fn record_null_test(&mut self, expr: &Expr, operator: &str) {
        let Some(column) = column_name(expr) else {
            return;
        };
        self.predicates.push(QueryPredicate {
            column,
            operator: operator.to_string(),
            operand: PredicateOperand::Null,
        });
    }

    fn operand(&mut self, expr: &Expr) -> PredicateOperand {
        match expr {
            Expr::Value(value) => match &value.value {
                Value::Placeholder(placeholder) => {
                    self.note_placeholder(placeholder);
                    let ordinal = positional_ordinal(placeholder).unwrap_or(0);
                    PredicateOperand::Parameter { ordinal }
                },
                Value::Null => PredicateOperand::Null,
                _ => PredicateOperand::Literal,
            },
            Expr::Nested(inner) => self.operand(inner),
            _ => PredicateOperand::Expression,
        }
    }
}

fn finish_parameters(drafts: &[ParamDraft]) -> Vec<QueryParameter> {
    let mut next = drafts.iter().map(|draft| draft.ordinal).max().unwrap_or(0);
    let mut parameters = drafts
        .iter()
        .map(|draft| {
            let ordinal = if draft.ordinal == 0 {
                next += 1;
                next
            } else {
                draft.ordinal
            };
            let compared_column = if draft.columns.len() == 1 {
                draft.columns.iter().next().cloned()
            } else {
                None
            };
            QueryParameter {
                ordinal,
                placeholder: draft.placeholder.clone(),
                compared_column,
            }
        })
        .collect::<Vec<_>>();
    parameters.sort_by_key(|param| param.ordinal);
    parameters
}

fn positional_ordinal(placeholder: &str) -> Option<u32> {
    placeholder.strip_prefix('$')?.parse().ok()
}

fn comparison_operator(rendered: &str) -> Option<&str> {
    match rendered {
        "=" | "<>" | "!=" | "<" | ">" | "<=" | ">=" => Some(rendered),
        _ => None,
    }
}

fn json_key_operand(expr: &Expr) -> bool {
    match expr {
        Expr::Value(value) => {
            matches!(value.value, Value::SingleQuotedString(_) | Value::Placeholder(_))
        },
        Expr::Nested(inner) => json_key_operand(inner),
        _ => false,
    }
}

fn column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => Some(ident.value.clone()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|part| part.value.clone()),
        Expr::Nested(inner) => column_name(inner),
        _ => None,
    }
}

fn is_aggregate_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "count"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "bool_and"
            | "bool_or"
            | "every"
            | "string_agg"
            | "array_agg"
            | "json_agg"
            | "jsonb_agg"
            | "bit_and"
            | "bit_or"
    )
}

fn map_parse_error(error: sqlparser::parser::ParserError) -> QueryPlanError {
    let message = error.to_string();
    if message.contains("->") {
        QueryPlanError::new("lambda array functions are not supported in named queries")
    } else {
        QueryPlanError::new(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::compile_contract_sql;

    fn school_catalog() -> ContractSnapshot {
        compile_contract_sql(
            r#"
            CREATE SCHEMA school;
            CREATE TABLE school.enrollments (
              enrollment_id TEXT PRIMARY KEY,
              id TEXT,
              seq_no BIGINT GENERATED ALWAYS AS IDENTITY,
              display_code TEXT GENERATED ALWAYS AS (enrollment_id) STORED,
              role TEXT NOT NULL DEFAULT 'student',
              tuition NUMERIC(12, 2) NOT NULL DEFAULT 0.00
            ) WITH (TYPE = 'SHARED');
            CREATE TABLE school.seats (
              seat_id TEXT PRIMARY KEY
            );
            "#,
            "public",
        )
        .unwrap()
    }

    #[test]
    fn t015_t016_binds_real_keys_parameters_cardinality_and_live_shape() {
        let catalog = school_catalog();
        let plan = analyze_named_query(
            "by_enrollment",
            "SELECT enrollment_id, role FROM school.enrollments WHERE enrollment_id = $1",
            Some(&catalog),
        )
        .unwrap();

        assert_eq!(plan.epoch, CURRENT_CONTRACT_EPOCH);
        assert_eq!(plan.cardinality, QueryCardinality::Many);
        assert!(plan.live.eligible);
        assert!(plan.key_complete);
        assert_eq!(plan.key_columns, vec!["enrollment_id".to_string()]);
        assert_eq!(plan.parameters.len(), 1);
        assert_eq!(plan.parameters[0].ordinal, 1);
        assert_eq!(plan.parameters[0].compared_column.as_deref(), Some("enrollment_id"));
        assert_eq!(plan.columns[1].type_name.as_deref(), Some("TEXT"));
        assert!(!plan.columns.iter().any(|column| column.name == "id"));
        plan.check_cancellation(false).unwrap();
        assert!(plan.check_cancellation(true).is_err());

        let limited = analyze_named_query(
            "one",
            "SELECT * FROM school.enrollments WHERE role = $1 LIMIT 1",
            Some(&catalog),
        )
        .unwrap();
        assert_eq!(limited.cardinality, QueryCardinality::AtMostOne);
        assert!(!limited.live.eligible);
        assert!(limited.key_complete);
        assert!(limited.projects_all_columns);
        assert!(limited
            .columns
            .iter()
            .any(|column| column.name == "display_code" && column.is_read_only));
        assert_eq!(limited.pagination.limit, Some(1));

        let partial =
            analyze_named_query("roles", "SELECT role FROM school.enrollments", Some(&catalog))
                .unwrap();
        assert!(!partial.key_complete);
        assert!(partial.key_columns.is_empty());

        let aliased = analyze_named_query(
            "aliased_key",
            "SELECT enrollment_id AS eid FROM school.enrollments",
            Some(&catalog),
        )
        .unwrap();
        assert_eq!(aliased.columns[0].name, "eid");
        assert!(aliased.key_complete);

        let expression_alias = analyze_named_query(
            "not_a_key",
            "SELECT role AS enrollment_id FROM school.enrollments",
            Some(&catalog),
        )
        .unwrap();
        assert!(!expression_alias.key_complete);

        let joined = analyze_named_query(
            "joined",
            "SELECT school.enrollments.enrollment_id FROM school.enrollments JOIN school.seats ON \
             school.enrollments.enrollment_id = school.seats.seat_id",
            Some(&catalog),
        )
        .unwrap();
        assert!(joined.features.joins);
        assert!(!joined.live.eligible);
        assert!(!joined.key_complete);

        let aggregate = analyze_named_query(
            "count_rows",
            "SELECT count(*) FROM school.enrollments",
            Some(&catalog),
        )
        .unwrap();
        assert_eq!(aggregate.cardinality, QueryCardinality::Scalar);
        assert!(aggregate.features.aggregates);
        assert!(!aggregate.live.eligible);
    }

    #[test]
    fn t015_preserves_quoted_identifiers_and_rejects_unknown_columns() {
        let catalog = school_catalog();
        let error = analyze_named_query(
            "missing",
            "SELECT missing_col FROM school.enrollments",
            Some(&catalog),
        )
        .unwrap_err();
        assert!(error.message.contains("missing_col"));

        let quoted =
            analyze_named_query("quoted", r#"SELECT "user-id" FROM school.enrollments"#, None)
                .unwrap();
        assert_eq!(quoted.columns[0].name, "user-id");
        assert!(quoted.sql.contains("\"user-id\""));
        assert!(!quoted.key_complete);
    }

    #[test]
    fn t015_rejects_lambda_syntax_and_empty_batches() {
        let catalog = school_catalog();
        let error = analyze_named_query(
            "lambda",
            "SELECT array_transform(role, x -> x) FROM school.enrollments",
            Some(&catalog),
        )
        .unwrap_err();
        assert!(error.message.contains("lambda"));

        let error = analyze_named_query(
            "two",
            "SELECT role FROM school.enrollments; SELECT role FROM school.seats",
            Some(&catalog),
        )
        .unwrap_err();
        assert!(error.message.contains("single SELECT"));

        let plan = analyze_named_query(
            "json",
            "SELECT role FROM school.enrollments WHERE role IS NULL AND enrollment_id -> 'kind' = \
             $2",
            Some(&catalog),
        )
        .unwrap();
        assert!(plan.parameters.iter().any(|param| param.ordinal == 2));
        assert!(plan.predicates.iter().any(|predicate| predicate.operator == "IS NULL"));
    }

    #[test]
    fn t016_json_arrow_stays_a_predicate_operand_not_a_lambda() {
        let plan = analyze_named_query(
            "json_filter",
            "SELECT enrollment_id FROM school.enrollments WHERE enrollment_id = payload -> 'id'",
            None,
        )
        .unwrap();
        assert!(
            plan.predicates.is_empty()
                || plan.predicates.iter().all(|predicate| predicate.operator != "->")
        );
        assert!(!plan.features.joins);
    }
}
