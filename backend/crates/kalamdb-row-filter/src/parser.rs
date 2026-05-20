use sqlparser::{
    ast::{Expr, SelectItem, SetExpr, Statement},
    dialect::GenericDialect,
    parser::Parser,
};

use crate::{RowFilter, RowFilterError, RowFilterResult};

pub fn parse_where_clause(where_clause: &str) -> RowFilterResult<RowFilter> {
    RowFilter::try_from_expr(parse_where_expr(where_clause)?)
}

pub fn parse_where_expr(where_clause: &str) -> RowFilterResult<Expr> {
    let dialect = GenericDialect {};
    let sql = format!("SELECT 1 WHERE {}", where_clause);
    let statements = Parser::parse_sql(&dialect, &sql)
        .map_err(|error| RowFilterError::Parse(error.to_string()))?;

    let [statement] = statements.as_slice() else {
        return Err(RowFilterError::InvalidSyntax);
    };

    let Statement::Query(query) = statement else {
        return Err(RowFilterError::InvalidSyntax);
    };

    let SetExpr::Select(select) = query.body.as_ref() else {
        return Err(RowFilterError::InvalidSyntax);
    };

    if select.projection.len() != 1 || !matches!(select.projection[0], SelectItem::UnnamedExpr(_)) {
        return Err(RowFilterError::InvalidSyntax);
    }

    select.selection.clone().ok_or(RowFilterError::InvalidSyntax)
}
