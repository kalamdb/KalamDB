//! INSERT ... VALUES row parsing.

use std::collections::BTreeMap;

use datafusion_common::ScalarValue;
use kalamdb_commons::models::rows::row::Row;
use sqlparser::ast::{Expr, Parens};

use super::literals::{expr_to_scalar, expr_to_scalar_with_params};

/// Convert INSERT VALUES rows into typed [`Row`] values.
///
/// Returns `Err` when expressions are not literals/placeholders so callers can
/// fall back to DataFusion.
pub fn values_to_rows(
    value_rows: &[Parens<Vec<Expr>>],
    column_names: &[String],
    params: &[ScalarValue],
) -> Result<Vec<Row>, &'static str> {
    if params.is_empty() {
        return literal_values_to_rows(value_rows, column_names);
    }

    parameterized_values_to_rows(value_rows, column_names, params)
}

fn literal_values_to_rows(
    value_rows: &[Parens<Vec<Expr>>],
    column_names: &[String],
) -> Result<Vec<Row>, &'static str> {
    let mut rows = Vec::with_capacity(value_rows.len());

    for value_row in value_rows {
        if value_row.content.len() != column_names.len() {
            return Err("column count mismatch");
        }

        let mut values = BTreeMap::new();
        for (expr, col_name) in value_row.content.iter().zip(column_names.iter()) {
            let scalar = expr_to_scalar(expr)?;
            values.insert(col_name.clone(), scalar);
        }
        rows.push(Row::new(values));
    }

    Ok(rows)
}

fn parameterized_values_to_rows(
    value_rows: &[Parens<Vec<Expr>>],
    column_names: &[String],
    params: &[ScalarValue],
) -> Result<Vec<Row>, &'static str> {
    let mut rows = Vec::with_capacity(value_rows.len());

    for value_row in value_rows {
        if value_row.content.len() != column_names.len() {
            return Err("column count mismatch");
        }

        let mut values = BTreeMap::new();
        for (expr, col_name) in value_row.content.iter().zip(column_names.iter()) {
            values.insert(col_name.clone(), expr_to_scalar_with_params(expr, params)?);
        }
        rows.push(Row::new(values));
    }

    Ok(rows)
}
