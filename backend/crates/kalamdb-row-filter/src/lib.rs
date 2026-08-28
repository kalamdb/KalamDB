//! Row-local SQL WHERE predicate parsing and evaluation.
//!
//! This crate intentionally handles only single-row predicates for hot paths
//! such as live query fanout and topic publishing. Bulk scans, DML filters,
//! MVCC visibility, and predicate pushdown should continue to use DataFusion's
//! physical expression machinery.

mod comparison;
mod error;
mod filter;
mod like;
mod parser;
mod predicate;
#[cfg(test)]
mod tests;
mod value;

pub use error::{RowFilterError, RowFilterResult};
pub use filter::RowFilter;
use kalamdb_commons::models::rows::Row;
pub use parser::{parse_where_clause, parse_where_expr};
pub use sqlparser::ast::Expr;

#[inline]
pub fn matches(filter: &RowFilter, row_data: &Row) -> RowFilterResult<bool> {
    filter.matches(row_data)
}
