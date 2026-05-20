use datafusion_common::ScalarValue;
use kalamdb_commons::as_f64;
use sqlparser::ast::BinaryOperator;

use crate::{value::scalar_as_str, RowFilterError, RowFilterResult};

#[derive(Clone, Copy, Debug)]
pub(crate) enum ComparisonOp {
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
}

impl ComparisonOp {
    pub(crate) fn from_binary_operator(operator: &BinaryOperator) -> RowFilterResult<Self> {
        match operator {
            BinaryOperator::Eq => Ok(Self::Eq),
            BinaryOperator::NotEq => Ok(Self::NotEq),
            BinaryOperator::Lt => Ok(Self::Lt),
            BinaryOperator::Gt => Ok(Self::Gt),
            BinaryOperator::LtEq => Ok(Self::LtEq),
            BinaryOperator::GtEq => Ok(Self::GtEq),
            _ => Err(RowFilterError::UnsupportedOperator(format!("{:?}", operator))),
        }
    }
}

pub(crate) fn compare_scalars(
    left: &ScalarValue,
    operator: ComparisonOp,
    right: &ScalarValue,
) -> RowFilterResult<bool> {
    if left.is_null() || right.is_null() {
        return Ok(false);
    }

    match operator {
        ComparisonOp::Eq => Ok(scalars_equal(left, right)),
        ComparisonOp::NotEq => Ok(!scalars_equal(left, right)),
        ComparisonOp::Lt => compare_numeric(left, right, |lhs, rhs| lhs < rhs),
        ComparisonOp::Gt => compare_numeric(left, right, |lhs, rhs| lhs > rhs),
        ComparisonOp::LtEq => compare_numeric(left, right, |lhs, rhs| lhs <= rhs),
        ComparisonOp::GtEq => compare_numeric(left, right, |lhs, rhs| lhs >= rhs),
    }
}

pub(crate) fn scalars_equal(left: &ScalarValue, right: &ScalarValue) -> bool {
    if left == right {
        return true;
    }

    if let (Some(lhs), Some(rhs)) = (scalar_as_str(left), scalar_as_str(right)) {
        return lhs == rhs;
    }

    if let (Some(lhs), Some(rhs)) = (as_f64(left), as_f64(right)) {
        return (lhs - rhs).abs() < f64::EPSILON;
    }

    false
}

fn compare_numeric<F>(left: &ScalarValue, right: &ScalarValue, compare: F) -> RowFilterResult<bool>
where
    F: FnOnce(f64, f64) -> bool,
{
    let left_num = as_f64(left).ok_or_else(|| {
        RowFilterError::InvalidOperation(format!("cannot convert {:?} to number", left))
    })?;
    let right_num = as_f64(right).ok_or_else(|| {
        RowFilterError::InvalidOperation(format!("cannot convert {:?} to number", right))
    })?;

    Ok(compare(left_num, right_num))
}
