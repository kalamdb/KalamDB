use kalamdb_commons::models::rows::Row;
use sqlparser::ast::Expr;

use crate::{parser::parse_where_expr, predicate::CompiledPredicate, RowFilterResult};

#[derive(Clone, Debug)]
pub struct RowFilter {
    predicate: CompiledPredicate,
}

impl RowFilter {
    pub fn parse(where_clause: &str) -> RowFilterResult<Self> {
        Self::try_from_expr(parse_where_expr(where_clause)?)
    }

    pub fn try_from_expr(expr: Expr) -> RowFilterResult<Self> {
        Ok(Self {
            predicate: CompiledPredicate::compile(&expr)?,
        })
    }

    #[inline]
    pub fn matches(&self, row_data: &Row) -> RowFilterResult<bool> {
        self.predicate.evaluate(row_data)
    }

    pub fn complexity(&self) -> usize {
        self.predicate.complexity()
    }
}
