//! SQL query parsing utilities for live queries and subscriptions.
//!
//! Uses sqlparser-rs for safe SQL parsing to prevent SQL injection attacks
//! and ensure proper handling of edge cases.
//!
//! General SELECT queries are executed by DataFusion 55 and may use SQL lambda array
//! functions. Subscription and live-query validation in this module intentionally
//! keeps a narrower surface.

use std::{fmt, ops::ControlFlow};

use kalamdb_commons::constants::SystemColumnNames;
use sqlparser::ast::{
    Expr, Function, FunctionArguments, GroupByExpr, Query, Select, SelectItem, SetExpr, Statement,
    TableFactor, TableWithJoins, Value, Visit, VisitMut, Visitor, VisitorMut,
};

use crate::{
    dialect::KalamDbDialect,
    parser::utils::{parse_sql_expression, parse_sql_statements},
};

/// Error type for query parsing
#[derive(Debug, Clone)]
pub enum QueryParseError {
    /// Failed to parse SQL
    ParseError(String),
    /// Invalid SQL for the operation
    InvalidSql(String),
}

impl QueryParseError {
    pub fn user_message(&self) -> &str {
        match self {
            Self::ParseError(message) | Self::InvalidSql(message) => message,
        }
    }

    pub fn into_user_message(self) -> String {
        match self {
            Self::ParseError(message) | Self::InvalidSql(message) => message,
        }
    }

    fn display_label(&self) -> &'static str {
        match self {
            Self::ParseError(_) => "Parse error",
            Self::InvalidSql(_) => "Invalid SQL",
        }
    }
}

impl fmt::Display for QueryParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.display_label(), self.user_message())
    }
}

impl std::error::Error for QueryParseError {}

/// Utilities for parsing SQL queries for live subscriptions
pub struct QueryParser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionQueryAnalysis {
    pub table_name: String,
    pub where_clause: Option<String>,
    pub projections: Option<Vec<String>>,
}

impl QueryParser {
    /// Parse and validate a subscription query once, returning the extracted parts.
    pub fn analyze_subscription_query(
        query: &str,
    ) -> Result<SubscriptionQueryAnalysis, QueryParseError> {
        let dialect = KalamDbDialect::default();
        let statements = parse_sql_statements(query, &dialect)
            .map_err(|e| QueryParseError::ParseError(e.to_string()))?;

        if statements.is_empty() {
            return Err(QueryParseError::InvalidSql("Empty SQL statement".to_string()));
        }

        if statements.len() != 1 {
            return Err(QueryParseError::InvalidSql(
                "Subscription query must contain exactly one SELECT statement".to_string(),
            ));
        }

        match &statements[0] {
            Statement::Query(query) => Self::analyze_subscription_query_ast(query),
            _ => Err(QueryParseError::InvalidSql(
                "Only SELECT queries are supported for subscriptions".to_string(),
            )),
        }
    }

    /// Validate that a subscription query uses only supported clauses.
    ///
    /// Supported shape:
    /// `SELECT <columns>|* FROM namespace.table [WHERE ...]`
    pub fn validate_subscription_query(query: &str) -> Result<(), QueryParseError> {
        Self::analyze_subscription_query(query).map(|_| ())
    }

    /// Extract table name from SQL query using sqlparser-rs
    ///
    /// This uses proper SQL parsing to safely extract the table name,
    /// preventing SQL injection attacks and handling edge cases like:
    /// - Quoted identifiers
    /// - Schema-qualified names (schema.table)
    /// - Complex FROM clauses
    ///
    /// # Security
    /// Uses sqlparser-rs instead of string manipulation to prevent
    /// SQL injection attacks in live query subscriptions.
    pub fn extract_table_name(query: &str) -> Result<String, QueryParseError> {
        Self::analyze_subscription_query(query).map(|analysis| analysis.table_name)
    }

    /// Extract table name from a parsed Query AST
    fn extract_table_from_query(query: &Query) -> Result<String, QueryParseError> {
        match query.body.as_ref() {
            SetExpr::Select(select) => Self::extract_table_from_select(select),
            _ => Err(QueryParseError::InvalidSql(
                "Unsupported query type for subscriptions".to_string(),
            )),
        }
    }

    /// Extract table name from a SELECT statement
    fn extract_table_from_select(select: &Select) -> Result<String, QueryParseError> {
        if select.from.is_empty() {
            return Err(QueryParseError::InvalidSql("No FROM clause in SELECT".to_string()));
        }

        let table_with_joins = &select.from[0];
        Self::extract_table_from_table_with_joins(table_with_joins)
    }

    /// Extract table name from TableWithJoins
    fn extract_table_from_table_with_joins(
        table_with_joins: &TableWithJoins,
    ) -> Result<String, QueryParseError> {
        match &table_with_joins.relation {
            TableFactor::Table { name, .. } => {
                use sqlparser::ast::ObjectNamePart;
                // Handle schema-qualified names (namespace.table)
                let mut parts = Vec::with_capacity(name.0.len());
                for part in &name.0 {
                    match part {
                        ObjectNamePart::Identifier(ident) => parts.push(ident.value.clone()),
                        _ => {
                            return Err(QueryParseError::InvalidSql(
                                "Subscription query table reference must be an identifier."
                                    .to_string(),
                            ));
                        },
                    }
                }

                if parts.is_empty() || parts.len() > 2 {
                    return Err(QueryParseError::InvalidSql(
                        "Subscription query table reference must be [namespace.]table.".to_string(),
                    ));
                }

                Ok(parts.join("."))
            },
            _ => Err(QueryParseError::InvalidSql(
                "Unsupported table type in FROM clause".to_string(),
            )),
        }
    }

    /// Extract WHERE clause from SQL query
    ///
    /// Returns the WHERE clause as a string for filter matching.
    /// Uses sqlparser-rs to safely parse and extract the clause.
    pub fn extract_where_clause(query: &str) -> Result<Option<String>, QueryParseError> {
        Self::analyze_subscription_query(query).map(|analysis| analysis.where_clause)
    }

    /// Extract WHERE clause from a parsed Query AST
    fn extract_where_from_query(query: &Query) -> Result<Option<String>, QueryParseError> {
        match query.body.as_ref() {
            SetExpr::Select(select) => {
                Ok(select.selection.as_ref().map(|expr| format!("{}", expr)))
            },
            _ => Ok(None),
        }
    }

    /// Extract projection columns from SQL query
    ///
    /// Returns a list of column names requested in the SELECT clause.
    /// Returns None for SELECT * queries.
    pub fn extract_projections(query: &str) -> Result<Option<Vec<String>>, QueryParseError> {
        Self::analyze_subscription_query(query).map(|analysis| analysis.projections)
    }

    /// Resolve placeholders like CURRENT_USER() in WHERE clause
    ///
    /// SECURITY: Parses the filter first and only replaces real AST nodes, not
    /// text inside string literals. The typed UserId is still escaped before it
    /// is emitted as a SQL string literal.
    pub fn resolve_where_clause_placeholders(
        where_clause: &str,
        user_id: &kalamdb_commons::models::UserId,
    ) -> Result<String, QueryParseError> {
        let escaped_user_id = user_id.as_str().replace('\'', "''");
        let dialect = KalamDbDialect::default();
        let mut expr = parse_sql_expression(where_clause, &dialect)
            .map_err(|error| QueryParseError::ParseError(error.to_string()))?;

        let mut visitor = CurrentUserPlaceholderResolver {
            replacement: Expr::value(Value::SingleQuotedString(escaped_user_id)),
        };
        let _ = VisitMut::visit(&mut expr, &mut visitor);
        Ok(expr.to_string())
    }

    /// Extract projection columns from a parsed Query AST
    fn extract_projections_from_query(
        query: &Query,
    ) -> Result<Option<Vec<String>>, QueryParseError> {
        match query.body.as_ref() {
            SetExpr::Select(select) => Self::extract_projections_from_select(select),
            _ => Ok(None),
        }
    }

    /// Extract projections from a SELECT statement
    fn extract_projections_from_select(
        select: &Select,
    ) -> Result<Option<Vec<String>>, QueryParseError> {
        let mut columns = Vec::new();
        let mut has_wildcard = false;

        for item in &select.projection {
            match item {
                SelectItem::Wildcard(_) => {
                    has_wildcard = true;
                    break;
                },
                SelectItem::UnnamedExpr(expr) => columns.push(Self::projection_column_name(expr)?),
                SelectItem::ExprWithAlias { .. } | SelectItem::ExprWithAliases { .. } => {
                    return Err(Self::unsupported_projection());
                },
                SelectItem::QualifiedWildcard(_, _) => return Err(Self::unsupported_projection()),
            }
        }

        if has_wildcard {
            Ok(None) // SELECT * or table.* returns None
        } else {
            Ok(Some(columns))
        }
    }

    fn projection_column_name(expr: &Expr) -> Result<String, QueryParseError> {
        match expr {
            Expr::Identifier(_) | Expr::CompoundIdentifier(_) => {
                Ok(Self::expr_to_column_name(expr))
            },
            _ => Err(Self::unsupported_projection()),
        }
    }

    fn unsupported_projection() -> QueryParseError {
        QueryParseError::InvalidSql(
            "Subscription query projections must be direct column references or '*'.".to_string(),
        )
    }

    /// Convert an expression to a column name string
    fn expr_to_column_name(expr: &Expr) -> String {
        match expr {
            Expr::Identifier(ident) => ident.value.clone(),
            Expr::CompoundIdentifier(parts) => {
                parts.iter().map(|p| p.value.clone()).collect::<Vec<_>>().join(".")
            },
            _ => format!("{}", expr),
        }
    }

    /// Parse parameterized WHERE clause expression
    ///
    /// Converts expressions like "user_id = $1" into a structured format
    /// for parameter substitution.
    pub fn parse_parameterized_expr(
        expr_str: &str,
    ) -> Result<(String, Vec<String>), QueryParseError> {
        let dialect = KalamDbDialect::default();
        parse_sql_expression(expr_str, &dialect)
            .map_err(|e| QueryParseError::ParseError(e.to_string()))?;

        // Extract parameters ($1, $2, etc.)
        let mut params = Vec::new();
        Self::extract_parameters_from_expr_str(expr_str, &mut params);

        Ok((expr_str.to_string(), params))
    }

    pub fn validate_row_filter_expr(expr_str: &str) -> Result<(), QueryParseError> {
        let dialect = KalamDbDialect::default();
        let expr = parse_sql_expression(expr_str, &dialect)
            .map_err(|e| QueryParseError::ParseError(e.to_string()))?;
        Self::validate_row_filter_ast(&expr)
    }

    /// Extract parameter placeholders from expression string
    fn extract_parameters_from_expr_str(expr_str: &str, params: &mut Vec<String>) {
        let re = regex::Regex::new(r"\$\d+").unwrap();
        for cap in re.captures_iter(expr_str) {
            if let Some(param) = cap.get(0) {
                let param_str = param.as_str().to_string();
                if !params.contains(&param_str) {
                    params.push(param_str);
                }
            }
        }
    }

    pub(crate) fn analyze_subscription_query_ast(
        query: &Query,
    ) -> Result<SubscriptionQueryAnalysis, QueryParseError> {
        Self::validate_subscription_query_ast(query)?;

        Ok(SubscriptionQueryAnalysis {
            table_name: Self::extract_table_from_query(query)?,
            where_clause: Self::extract_where_from_query(query)?,
            projections: Self::extract_projections_from_query(query)?,
        })
    }

    fn validate_subscription_query_ast(query: &Query) -> Result<(), QueryParseError> {
        if query.with.is_some() {
            return Err(QueryParseError::InvalidSql(
                "Subscription query does not support WITH clauses. Only SELECT ... FROM ... \
                 [WHERE ...] is supported."
                    .to_string(),
            ));
        }

        if query.order_by.is_some() {
            return Err(QueryParseError::InvalidSql(
                "Subscription query does not support ORDER BY. Only SELECT ... FROM ... [WHERE \
                 ...] is supported."
                    .to_string(),
            ));
        }

        if query.limit_clause.is_some()
            || query.fetch.is_some()
            || !query.locks.is_empty()
            || query.for_clause.is_some()
            || query.settings.is_some()
            || query.format_clause.is_some()
            || !query.pipe_operators.is_empty()
        {
            return Err(QueryParseError::InvalidSql(
                "Subscription query supports only SELECT ... FROM ... [WHERE ...].".to_string(),
            ));
        }

        match query.body.as_ref() {
            SetExpr::Select(select) => Self::validate_subscription_select(select),
            _ => Err(QueryParseError::InvalidSql(
                "Subscription query supports only simple SELECT statements.".to_string(),
            )),
        }
    }

    fn validate_subscription_select(select: &Select) -> Result<(), QueryParseError> {
        if select.distinct.is_some()
            || select.select_modifiers.is_some()
            || select.top.is_some()
            || select.exclude.is_some()
            || select.into.is_some()
            || !select.lateral_views.is_empty()
            || select.prewhere.is_some()
            || !select.connect_by.is_empty()
            || !select.cluster_by.is_empty()
            || !select.distribute_by.is_empty()
            || !select.sort_by.is_empty()
            || select.having.is_some()
            || !select.named_window.is_empty()
            || select.qualify.is_some()
            || select.value_table_mode.is_some()
        {
            return Err(QueryParseError::InvalidSql(
                "Subscription query supports only SELECT ... FROM ... [WHERE ...].".to_string(),
            ));
        }

        if Self::has_group_by(&select.group_by) {
            return Err(QueryParseError::InvalidSql(
                "Subscription query does not support GROUP BY. Only SELECT ... FROM ... [WHERE \
                 ...] is supported."
                    .to_string(),
            ));
        }

        if select.from.len() != 1 {
            return Err(QueryParseError::InvalidSql(
                "Subscription query requires exactly one table in the FROM clause.".to_string(),
            ));
        }

        let table_with_joins = &select.from[0];
        if !table_with_joins.joins.is_empty() {
            return Err(QueryParseError::InvalidSql(
                "Subscription query does not support JOIN clauses. Only SELECT ... FROM ... \
                 [WHERE ...] is supported."
                    .to_string(),
            ));
        }

        match &table_with_joins.relation {
            TableFactor::Table { alias, .. } => {
                if alias.is_some() {
                    return Err(QueryParseError::InvalidSql(
                        "Subscription query does not support table aliases.".to_string(),
                    ));
                }
            },
            _ => {
                return Err(QueryParseError::InvalidSql(
                    "Subscription query requires a direct table reference in FROM.".to_string(),
                ));
            },
        }

        Self::validate_subscription_projection_items(&select.projection)?;

        if let Some(selection) = &select.selection {
            Self::validate_row_filter_ast(selection)?;
        }

        Ok(())
    }

    fn validate_subscription_projection_items(
        projection: &[SelectItem],
    ) -> Result<(), QueryParseError> {
        for item in projection {
            let column_name = match item {
                SelectItem::Wildcard(_) => continue,
                SelectItem::UnnamedExpr(expr) => Some(Self::projection_column_name(expr)?),
                SelectItem::ExprWithAlias { .. }
                | SelectItem::ExprWithAliases { .. }
                | SelectItem::QualifiedWildcard(..) => {
                    return Err(Self::unsupported_projection());
                },
            };

            if let Some(column_name) = column_name {
                let base_name = column_name.rsplit('.').next().unwrap_or(&column_name);
                if base_name.eq_ignore_ascii_case(SystemColumnNames::SEQ)
                    || base_name.eq_ignore_ascii_case(SystemColumnNames::DELETED)
                {
                    return Err(QueryParseError::InvalidSql(format!(
                        "Subscription query cannot project system columns '{}' or '{}'.",
                        SystemColumnNames::SEQ,
                        SystemColumnNames::DELETED
                    )));
                }
            }
        }

        Ok(())
    }

    fn has_group_by(group_by: &GroupByExpr) -> bool {
        match group_by {
            GroupByExpr::All(_) => true,
            GroupByExpr::Expressions(expressions, _) => !expressions.is_empty(),
        }
    }

    fn validate_row_filter_ast(expr: &Expr) -> Result<(), QueryParseError> {
        let mut visitor = SubscriptionWhereExprValidator;
        match expr.visit(&mut visitor) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(error) => Err(error),
        }
    }
}

struct CurrentUserPlaceholderResolver {
    replacement: Expr,
}

impl VisitorMut for CurrentUserPlaceholderResolver {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if is_current_user_placeholder(expr) {
            *expr = self.replacement.clone();
        }
        ControlFlow::Continue(())
    }
}

fn is_current_user_placeholder(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(ident) => ident.value.eq_ignore_ascii_case("CURRENT_USER"),
        Expr::Function(function) => is_current_user_function(function),
        _ => false,
    }
}

fn is_current_user_function(function: &Function) -> bool {
    let name = function.name.to_string();
    if !name.eq_ignore_ascii_case("CURRENT_USER") && !name.eq_ignore_ascii_case("CURRENT_USER_ID") {
        return false;
    }

    matches!(&function.args, FunctionArguments::None)
        || matches!(&function.args, FunctionArguments::List(args) if args.args.is_empty())
}

struct SubscriptionWhereExprValidator;

impl Visitor for SubscriptionWhereExprValidator {
    type Break = QueryParseError;

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Subquery(_) | Expr::Exists { .. } | Expr::InSubquery { .. } => {
                ControlFlow::Break(QueryParseError::InvalidSql(
                    "Subscription WHERE clause does not support subqueries; only row-local \
                     predicates are allowed."
                        .to_string(),
                ))
            },
            _ => ControlFlow::Continue(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_table_name_simple() {
        let result = QueryParser::extract_table_name("SELECT * FROM users");
        assert_eq!(result.unwrap(), "users");
    }

    #[test]
    fn test_extract_table_name_qualified() {
        let result = QueryParser::extract_table_name("SELECT * FROM chat.messages");
        assert_eq!(result.unwrap(), "chat.messages");
    }

    #[test]
    fn test_extract_where_clause() {
        let result = QueryParser::extract_where_clause("SELECT * FROM users WHERE id = 1").unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().contains("id"));
    }

    #[test]
    fn test_extract_projections_wildcard() {
        let result = QueryParser::extract_projections("SELECT * FROM users").unwrap();
        assert!(result.is_none()); // Wildcard returns None
    }

    #[test]
    fn test_extract_projections_specific() {
        let result = QueryParser::extract_projections("SELECT id, name, email FROM users").unwrap();
        assert!(result.is_some());
        let cols = result.unwrap();
        assert_eq!(cols, vec!["id", "name", "email"]);
    }

    #[test]
    fn test_validate_subscription_query_accepts_select_where() {
        QueryParser::validate_subscription_query(
            "SELECT id, name FROM chat.messages WHERE conversation_id = 1",
        )
        .unwrap();
    }

    #[test]
    fn test_validate_subscription_query_rejects_order_by() {
        let err = QueryParser::validate_subscription_query(
            "SELECT id FROM chat.messages WHERE conversation_id = 1 ORDER BY id",
        )
        .unwrap_err();

        assert!(err.to_string().contains("ORDER BY"));
    }

    #[test]
    fn test_validate_subscription_query_rejects_group_by() {
        let err = QueryParser::validate_subscription_query(
            "SELECT user_id FROM chat.messages GROUP BY user_id",
        )
        .unwrap_err();

        assert!(err.to_string().contains("GROUP BY"));
    }

    #[test]
    fn test_validate_subscription_query_rejects_seq_projection() {
        let err =
            QueryParser::validate_subscription_query("SELECT _seq FROM chat.messages").unwrap_err();

        assert!(err.to_string().contains(SystemColumnNames::SEQ));
    }

    #[test]
    fn test_validate_subscription_query_rejects_deleted_projection() {
        let err = QueryParser::validate_subscription_query(
            "SELECT chat.messages._deleted FROM chat.messages",
        )
        .unwrap_err();

        assert!(err.to_string().contains(SystemColumnNames::DELETED));
    }

    #[test]
    fn test_validate_subscription_query_rejects_computed_projection() {
        let err = QueryParser::validate_subscription_query(
            "SELECT CONCAT(user_id, '-x') FROM chat.messages",
        )
        .unwrap_err();

        assert!(err.to_string().contains("direct column references"));
    }

    #[test]
    fn test_validate_subscription_query_rejects_projection_alias() {
        let err =
            QueryParser::validate_subscription_query("SELECT user_id AS actor FROM chat.messages")
                .unwrap_err();

        assert!(err.to_string().contains("direct column references"));
    }

    #[test]
    fn test_validate_subscription_query_rejects_table_alias() {
        let err = QueryParser::validate_subscription_query(
            "SELECT m.user_id FROM chat.messages AS m WHERE m.id = 1",
        )
        .unwrap_err();

        assert!(err.to_string().contains("table aliases"));
    }

    #[test]
    fn test_validate_subscription_query_rejects_where_subqueries() {
        let err = QueryParser::validate_subscription_query(
            "SELECT id FROM chat.messages WHERE EXISTS (SELECT 1 FROM system.users)",
        )
        .unwrap_err();

        assert!(err.to_string().contains("does not support subqueries"));
    }

    #[test]
    fn test_validate_row_filter_expr_rejects_trailing_tokens() {
        let err = QueryParser::validate_row_filter_expr("user_id = 'alice'; DROP TABLE users")
            .unwrap_err();

        assert!(err.to_string().contains("trailing tokens"));
    }

    #[test]
    fn test_resolve_where_clause_placeholders_handles_keyword_form() {
        let resolved = QueryParser::resolve_where_clause_placeholders(
            "owner_id = CURRENT_USER AND body = 'CURRENT_USER() literal'",
            &kalamdb_commons::models::UserId::from("user_1"),
        )
        .expect("placeholder resolution");

        assert_eq!(resolved, "owner_id = 'user_1' AND body = 'CURRENT_USER() literal'");
    }

    #[test]
    fn test_analyze_subscription_query_extracts_all_parts() {
        let analysis = QueryParser::analyze_subscription_query(
            "SELECT id, name FROM chat.messages WHERE conversation_id = 1",
        )
        .unwrap();

        assert_eq!(analysis.table_name, "chat.messages");
        assert_eq!(analysis.where_clause, Some("conversation_id = 1".to_string()));
        assert_eq!(analysis.projections, Some(vec!["id".to_string(), "name".to_string()]));
    }

    // ========================================================================
    // Security Tests: Row-Filter Hardening & Placeholder Safety
    //
    // These tests act as tripwires: if a guard is removed or bypassed the
    // test turns red. Do not relax assertions without a security review.
    // ========================================================================

    // ── validate_row_filter_expr — subquery injection ────────────────────────

    #[test]
    fn test_security_filter_rejects_in_subquery() {
        let err = QueryParser::validate_row_filter_expr(
            "user_id IN (SELECT id FROM system.users WHERE role = 'admin')",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("subqueries"),
            "IN (SELECT …) in row filter must be blocked; got: {err}"
        );
    }

    #[test]
    fn test_security_filter_rejects_scalar_subquery() {
        let err = QueryParser::validate_row_filter_expr(
            "role = (SELECT role FROM system.users WHERE id = 'attacker')",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("subqueries"),
            "Scalar subquery in row filter must be blocked; got: {err}"
        );
    }

    #[test]
    fn test_security_filter_rejects_exists_subquery() {
        let err = QueryParser::validate_row_filter_expr(
            "EXISTS (SELECT 1 FROM system.api_keys WHERE scope = 'superadmin')",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("subqueries"),
            "EXISTS subquery must be blocked; got: {err}"
        );
    }

    #[test]
    fn test_security_filter_rejects_not_exists_subquery() {
        let err = QueryParser::validate_row_filter_expr(
            "NOT EXISTS (SELECT 1 FROM security.blocklist WHERE blocked_id = user_id)",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("subqueries"),
            "NOT EXISTS subquery must be blocked; got: {err}"
        );
    }

    #[test]
    fn test_security_filter_rejects_in_subquery_api_keys() {
        // Attempts to correlate user data with a secret table to grant/deny
        // visibility of rows based on privileged information.
        let err = QueryParser::validate_row_filter_expr(
            "token IN (SELECT api_key FROM system.api_keys WHERE scope = 'superadmin')",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("subqueries"),
            "IN (SELECT …) on privileged table must be blocked; got: {err}"
        );
    }

    #[test]
    fn test_security_filter_rejects_nested_subquery_chain() {
        // Doubly nested subquery: each level individually breaches the guard.
        let err = QueryParser::validate_row_filter_expr(
            "id IN (SELECT id FROM app.sessions WHERE token = (SELECT token FROM system.users LIMIT 1))",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("subqueries"),
            "Nested subquery chain must be blocked; got: {err}"
        );
    }

    // ── validate_row_filter_expr — trailing-token injection ──────────────────

    #[test]
    fn test_security_filter_rejects_stacked_drop() {
        let err =
            QueryParser::validate_row_filter_expr("user_id = 'alice'; DROP TABLE system.users")
                .unwrap_err();
        assert!(
            err.to_string().contains("trailing tokens"),
            "Stacked DROP must be rejected as trailing tokens; got: {err}"
        );
    }

    #[test]
    fn test_security_filter_rejects_stacked_update_after_predicate() {
        let err = QueryParser::validate_row_filter_expr(
            "active = true; UPDATE system.users SET role = 'admin' WHERE 1=1",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("trailing tokens"),
            "Stacked UPDATE must be rejected; got: {err}"
        );
    }

    #[test]
    fn test_security_filter_rejects_null_byte_like_separator() {
        // The raw string `1=1 --` ends with a comment; the comment is consumed
        // by the tokeniser so the expression is valid but harmless. This test
        // verifies the comment does NOT allow SQL injection by confirming the
        // expression parses cleanly (no trailing tokens survive the comment).
        let result = QueryParser::validate_row_filter_expr("user_id = 'alice' -- harmless comment");
        assert!(
            result.is_ok(),
            "A trailing comment is safe and must not be rejected; got: {result:?}"
        );
    }

    // ── resolve_where_clause_placeholders — placeholder safety ───────────────

    #[test]
    fn test_security_placeholder_string_literal_not_treated_as_placeholder() {
        // The value 'CURRENT_USER' inside a SQL string literal must be
        // preserved as-is. Only AST identifier / function nodes are replaced.
        let resolved = QueryParser::resolve_where_clause_placeholders(
            "category = 'CURRENT_USER' AND owner_id = CURRENT_USER",
            &kalamdb_commons::models::UserId::from("eve"),
        )
        .expect("placeholder resolution");
        assert!(
            resolved.contains("'CURRENT_USER'"),
            "String literal must NOT be replaced; got: {resolved}"
        );
        assert!(
            resolved.contains("'eve'"),
            "AST CURRENT_USER must be replaced with user id; got: {resolved}"
        );
        // The user id literal must appear exactly once — not substituted into
        // the adjacent string literal too.
        assert_eq!(
            resolved.matches("'eve'").count(),
            1,
            "User id must appear exactly once; got: {resolved}"
        );
    }

    #[test]
    fn test_security_placeholder_all_occurrences_in_complex_predicate() {
        // Every CURRENT_USER node in the AST must be replaced, not only the
        // first. An adversary could craft `owner = CURRENT_USER AND viewer =
        // CURRENT_USER` hoping the second instance escapes substitution.
        let resolved = QueryParser::resolve_where_clause_placeholders(
            "owner_id = CURRENT_USER AND delegate_id = CURRENT_USER",
            &kalamdb_commons::models::UserId::from("carol"),
        )
        .expect("placeholder resolution");
        assert_eq!(
            resolved.matches("'carol'").count(),
            2,
            "Both CURRENT_USER nodes must be replaced; got: {resolved}"
        );
        assert!(
            !resolved.contains("CURRENT_USER"),
            "No unresolved CURRENT_USER must remain; got: {resolved}"
        );
    }

    #[test]
    fn test_security_placeholder_current_user_id_alias_replaced() {
        // CURRENT_USER_ID() is a KalamDB alias; it must resolve identically
        // to CURRENT_USER() so users cannot bypass replacement by spelling
        // it differently.
        let resolved = QueryParser::resolve_where_clause_placeholders(
            "owner_id = CURRENT_USER_ID()",
            &kalamdb_commons::models::UserId::from("frank"),
        )
        .expect("placeholder resolution");
        assert!(
            resolved.contains("'frank'"),
            "CURRENT_USER_ID() must be replaced; got: {resolved}"
        );
        assert!(
            !resolved.to_uppercase().contains("CURRENT_USER"),
            "No unresolved placeholder must remain; got: {resolved}"
        );
    }

    #[test]
    fn test_security_placeholder_unparseable_filter_rejected() {
        let broken = "user_id = CURRENT_USER AND (";
        let err = QueryParser::resolve_where_clause_placeholders(
            broken,
            &kalamdb_commons::models::UserId::from("grace"),
        )
        .expect_err("unparseable filter must be rejected");
        assert!(
            matches!(err, QueryParseError::ParseError(_)),
            "Unparseable filter must fail parse; got: {err:?}"
        );
    }

    #[test]
    fn test_security_placeholder_current_user_in_nested_or_expression() {
        // CURRENT_USER buried inside an OR sub-expression must still be
        // replaced; the AST visitor descends the full expression tree.
        let resolved = QueryParser::resolve_where_clause_placeholders(
            "(owner_id = CURRENT_USER() OR shared = true) AND active = true",
            &kalamdb_commons::models::UserId::from("heidi"),
        )
        .expect("placeholder resolution");
        assert!(
            resolved.contains("'heidi'"),
            "CURRENT_USER() inside OR must be replaced; got: {resolved}"
        );
        assert!(
            !resolved.to_uppercase().contains("CURRENT_USER"),
            "No unresolved CURRENT_USER must remain; got: {resolved}"
        );
    }

    #[test]
    fn test_security_filter_accepts_safe_boolean_predicate() {
        // A plain boolean predicate (no subquery, no trailing tokens) is a
        // legitimate row-local filter and must NOT be rejected.
        QueryParser::validate_row_filter_expr("active = true AND role = 'user'")
            .expect("Simple boolean predicate must be accepted");
    }

    #[test]
    fn test_security_filter_accepts_in_literal_list() {
        // IN with a literal value list is safe (no correlated table lookup).
        QueryParser::validate_row_filter_expr("status IN ('pending', 'active', 'paused')")
            .expect("IN with literal list must be accepted");
    }
}
