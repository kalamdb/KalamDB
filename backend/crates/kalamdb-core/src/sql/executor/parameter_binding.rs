//! Parameter binding and validation for SQL execution
//!
//! This module handles:
//! - Parameter validation (max 65535 params, 512KB per param)
//! - DataFusion LogicalPlan placeholder replacement ($1, $2, ...)
//! - ScalarValue type checking

use arrow::datatypes::{DataType, Field};
use datafusion::{
    common::{
        tree_node::{Transformed, TransformedResult, TreeNode},
        ParamValues,
    },
    logical_expr::{Expr, LogicalPlan},
    scalar::ScalarValue,
};
use kalamdb_commons::conversions::arrow_json_conversion::coerce_scalar_to_field;

use crate::error::KalamDbError;

/// Maximum number of parameters allowed per statement.
///
/// PostgreSQL's extended protocol encodes the parameter count as an int16,
/// so 65535 is the protocol maximum.
const MAX_PARAMS: usize = 65535;

/// Maximum size per individual parameter (512KB)
const MAX_PARAM_SIZE_BYTES: usize = 512 * 1024;

/// Validate parameter count and sizes before execution
pub fn validate_params(params: &[ScalarValue]) -> Result<(), KalamDbError> {
    // Check parameter count
    if params.len() > MAX_PARAMS {
        return Err(KalamDbError::ParamCountExceeded {
            max: MAX_PARAMS,
            actual: params.len(),
        });
    }

    // Check individual parameter sizes
    for (idx, param) in params.iter().enumerate() {
        let size = estimate_scalar_value_size(param);
        if size > MAX_PARAM_SIZE_BYTES {
            return Err(KalamDbError::ParamSizeExceeded {
                index: idx,
                max_bytes: MAX_PARAM_SIZE_BYTES,
                actual_bytes: size,
            });
        }
    }

    Ok(())
}

/// Estimate the memory size of a ScalarValue
fn estimate_scalar_value_size(value: &ScalarValue) -> usize {
    kalamdb_commons::estimate_scalar_value_size(value)
}

/// Replace placeholders ($1, $2, ...) in LogicalPlan with ScalarValue literals
///
/// Uses DataFusion's built-in `with_param_values()` method which handles:
/// - Type inference for placeholders
/// - Subquery traversal
/// - Schema updates after replacement
///
/// # Arguments
/// * `plan` - LogicalPlan to process
/// * `params` - Parameter values indexed from 0 (placeholder $1 = params[0])
///
/// # Returns
/// * `Ok(LogicalPlan)` - Plan with all placeholders replaced
/// * `Err(KalamDbError)` - If placeholder index is out of bounds or type mismatch
///
/// # Example
/// ```ignore
/// use datafusion::prelude::*;
/// use datafusion::scalar::ScalarValue;
///
/// let plan = ctx.sql("SELECT * FROM users WHERE id = $1").await?.into_optimized_plan()?;
/// let params = vec![ScalarValue::Int64(Some(42))];
/// let bound_plan = replace_placeholders_in_plan(plan, &params)?;
/// ```
pub fn replace_placeholders_in_plan(
    plan: LogicalPlan,
    params: &[ScalarValue],
) -> Result<LogicalPlan, KalamDbError> {
    // If no params, return plan unchanged
    if params.is_empty() {
        return Ok(plan);
    }

    // Use DataFusion's built-in method which handles:
    // - Type inference via infer_placeholder_types
    // - Subquery traversal
    // - Schema updates after replacement
    //
    // Wire clients send UUID values as text. The placeholder is inferred as
    // FixedSizeBinary(16), and DataFusion will not compare that to Utf8.
    let params = coerce_params_to_placeholder_fields(&plan, params)?;
    let param_values: ParamValues = params.into();
    plan.with_param_values(param_values)
        .map_err(|error| KalamDbError::ParameterBindingError {
            message: error.to_string(),
        })
}

fn coerce_params_to_placeholder_fields(
    plan: &LogicalPlan,
    params: &[ScalarValue],
) -> Result<Vec<ScalarValue>, KalamDbError> {
    let fields =
        plan.get_parameter_fields()
            .map_err(|error| KalamDbError::ParameterBindingError {
                message: error.to_string(),
            })?;
    let mut coerced = params.to_vec();
    for (id, field) in fields {
        let Some(field) = field else {
            continue;
        };
        let Some(index) = placeholder_index(&id) else {
            continue;
        };
        let Some(value) = coerced.get(index).cloned() else {
            continue;
        };
        coerced[index] = coerce_bound_parameter(value, field.as_ref())?;
    }
    Ok(coerced)
}

/// PostgreSQL accepts a text parameter where a `uuid` column is expected.
/// DataFusion stores UUID as `FixedSizeBinary(16)` and rejects `uuid = utf8`.
fn coerce_bound_parameter(value: ScalarValue, field: &Field) -> Result<ScalarValue, KalamDbError> {
    if !matches!(field.data_type(), DataType::FixedSizeBinary(16)) {
        return Ok(value);
    }
    if value.data_type() == *field.data_type() {
        return Ok(value);
    }
    coerce_scalar_to_field(value, field)
        .map_err(|message| KalamDbError::ParameterBindingError { message })
}

fn placeholder_index(id: &str) -> Option<usize> {
    let digits = id.strip_prefix('$').unwrap_or(id);
    let index = digits.parse::<usize>().ok()?;
    index.checked_sub(1)
}

/// Replace `$n` placeholders in a single expression. Used by the point-get fast
/// path so a PK bind does not clone the full cached `LogicalPlan`.
pub fn bind_placeholders_in_expr(expr: Expr, params: &[ScalarValue]) -> Result<Expr, KalamDbError> {
    if params.is_empty() {
        return Ok(expr);
    }

    let mut bind_error = None;
    let bound = expr.transform_up(|node| {
        let Expr::Placeholder(placeholder) = node else {
            return Ok(Transformed::no(node));
        };
        let Some(index) = placeholder_index(&placeholder.id) else {
            return Ok(Transformed::no(Expr::Placeholder(placeholder)));
        };
        let Some(mut value) = params.get(index).cloned() else {
            bind_error = Some(KalamDbError::ParameterBindingError {
                message: format!(
                    "placeholder {} is out of range for {} parameters",
                    placeholder.id,
                    params.len()
                ),
            });
            return Ok(Transformed::no(Expr::Placeholder(placeholder)));
        };
        if let Some(field) = placeholder.field.clone() {
            match coerce_bound_parameter(value, field.as_ref()) {
                Ok(coerced) => value = coerced,
                Err(error) => {
                    bind_error = Some(error);
                    return Ok(Transformed::no(Expr::Placeholder(placeholder)));
                },
            }
        }
        Ok(Transformed::yes(Expr::Literal(value, None)))
    });
    if let Some(error) = bind_error {
        return Err(error);
    }
    bound.data().map_err(|error| KalamDbError::ParameterBindingError {
        message: error.to_string(),
    })
}

/// Highest `$n` placeholder index in SQL (0 when none).
pub fn max_placeholder_index(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut index = 0usize;
    let mut max = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'$' {
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index > start {
                if let Ok(number) =
                    std::str::from_utf8(&bytes[start..index]).unwrap_or("").parse::<usize>()
                {
                    max = max.max(number);
                }
            }
        } else {
            index += 1;
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_params_count() {
        let params = vec![ScalarValue::Int64(Some(1)); 94];
        assert!(validate_params(&params).is_ok());

        let params = vec![ScalarValue::Int64(Some(1)); 65536];
        assert!(matches!(validate_params(&params), Err(KalamDbError::ParamCountExceeded { .. })));
    }

    #[test]
    fn test_validate_params_size() {
        // Valid: under size limit
        let small_string = "x".repeat(1000);
        let params = vec![ScalarValue::Utf8(Some(small_string))];
        assert!(validate_params(&params).is_ok());

        // Invalid: over size limit (600KB)
        let large_string = "x".repeat(600_000);
        let params = vec![ScalarValue::Utf8(Some(large_string))];
        assert!(matches!(validate_params(&params), Err(KalamDbError::ParamSizeExceeded { .. })));
    }

    #[test]
    fn test_estimate_scalar_value_size() {
        assert_eq!(estimate_scalar_value_size(&ScalarValue::Int64(Some(123))), 8);
        assert_eq!(
            estimate_scalar_value_size(&ScalarValue::Utf8(Some("hello".to_string()))),
            24 + 5
        );
        assert_eq!(estimate_scalar_value_size(&ScalarValue::Null), 1);
    }

    #[test]
    fn max_placeholder_index_finds_highest_number() {
        assert_eq!(
            max_placeholder_index(
                "SELECT * FROM t WHERE table_schema = $1 AND table_name = $2 ORDER BY $1"
            ),
            2
        );
    }

    #[test]
    fn text_uuid_parameter_coerces_to_fixed_size_binary() {
        let field = Field::new("id", DataType::FixedSizeBinary(16), true);
        let coerced = coerce_bound_parameter(
            ScalarValue::Utf8(Some("abf8a154-5b1c-4a46-ac9c-7300570f4f17".to_string())),
            &field,
        )
        .expect("uuid text");
        match coerced {
            ScalarValue::FixedSizeBinary(16, Some(bytes)) => assert_eq!(bytes.len(), 16),
            other => panic!("expected uuid bytes, got {other:?}"),
        }
    }

    #[test]
    fn bind_placeholders_in_expr_replaces_dollar_one() {
        use datafusion::logical_expr::expr::Placeholder;

        let expr = Expr::Placeholder(Placeholder::new_with_field("$1".to_string(), None));
        let bound =
            bind_placeholders_in_expr(expr, &[ScalarValue::Int64(Some(42))]).expect("bind $1");
        assert_eq!(bound, Expr::Literal(ScalarValue::Int64(Some(42)), None));
    }

    #[test]
    fn bind_placeholders_in_expr_out_of_range_errors() {
        use datafusion::logical_expr::expr::Placeholder;

        let expr = Expr::Placeholder(Placeholder::new_with_field("$2".to_string(), None));
        let err = bind_placeholders_in_expr(expr, &[ScalarValue::Int64(Some(1))]).unwrap_err();
        assert!(matches!(err, KalamDbError::ParameterBindingError { .. }));
    }
}
