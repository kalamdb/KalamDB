use datafusion_common::ScalarValue;
use kalamdb_commons::models::rows::Row;
use sqlparser::ast::{BinaryOperator, Expr, UnaryOperator};

use crate::{
    comparison::{compare_scalars, ComparisonOp},
    like::LikePredicate,
    value::ValueExpr,
    RowFilterError, RowFilterResult,
};

const MAX_FILTER_DEPTH: usize = 64;
const MAX_IN_LIST_VALUES: usize = 1024;

#[derive(Clone, Debug)]
pub(crate) struct CompiledPredicate {
    predicate:  Predicate,
    complexity: usize,
}

impl CompiledPredicate {
    pub(crate) fn compile(expr: &Expr) -> RowFilterResult<Self> {
        Predicate::compile(expr, 0)
    }

    pub(crate) fn evaluate(&self, row_data: &Row) -> RowFilterResult<bool> {
        self.predicate.evaluate(row_data)
    }

    pub(crate) fn complexity(&self) -> usize {
        self.complexity
    }
}

#[derive(Clone, Debug)]
enum Predicate {
    Bool(ValueExpr),
    Not(Box<CompiledPredicate>),
    And(Box<CompiledPredicate>, Box<CompiledPredicate>),
    Or(Box<CompiledPredicate>, Box<CompiledPredicate>),
    Comparison {
        left:     ValueExpr,
        operator: ComparisonOp,
        right:    ValueExpr,
    },
    Between {
        value:   ValueExpr,
        low:     ValueExpr,
        high:    ValueExpr,
        negated: bool,
    },
    InList {
        value:   ValueExpr,
        list:    Box<[ValueExpr]>,
        negated: bool,
    },
    IsNull {
        value:   ValueExpr,
        negated: bool,
    },
    IsTrue {
        value:   ValueExpr,
        negated: bool,
    },
    IsFalse {
        value:   ValueExpr,
        negated: bool,
    },
    Like(LikePredicate),
}

impl Predicate {
    fn compile(expr: &Expr, depth: usize) -> RowFilterResult<CompiledPredicate> {
        if depth > MAX_FILTER_DEPTH {
            return Err(RowFilterError::InvalidOperation(format!(
                "row filter expression exceeds max depth of {}",
                MAX_FILTER_DEPTH
            )));
        }

        let compiled = match expr {
            Expr::Nested(inner) => return Self::compile(inner, depth + 1),
            Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr,
            } => {
                let inner = Self::compile(expr, depth + 1)?;
                CompiledPredicate {
                    complexity: inner.complexity + 1,
                    predicate:  Predicate::Not(Box::new(inner)),
                }
            },
            Expr::UnaryOp { op, .. } => {
                return Err(RowFilterError::UnsupportedOperator(format!("{:?}", op)))
            },
            Expr::BinaryOp { left, op, right } => match op {
                BinaryOperator::And => {
                    let left = Self::compile(left, depth + 1)?;
                    let right = Self::compile(right, depth + 1)?;
                    CompiledPredicate {
                        complexity: left.complexity + right.complexity + 1,
                        predicate:  Predicate::And(Box::new(left), Box::new(right)),
                    }
                },
                BinaryOperator::Or => {
                    let left = Self::compile(left, depth + 1)?;
                    let right = Self::compile(right, depth + 1)?;
                    CompiledPredicate {
                        complexity: left.complexity + right.complexity + 1,
                        predicate:  Predicate::Or(Box::new(left), Box::new(right)),
                    }
                },
                operator => CompiledPredicate {
                    complexity: 1,
                    predicate:  Predicate::Comparison {
                        left:     ValueExpr::compile(left)?,
                        operator: ComparisonOp::from_binary_operator(operator)?,
                        right:    ValueExpr::compile(right)?,
                    },
                },
            },
            Expr::Between {
                expr,
                negated,
                low,
                high,
            } => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::Between {
                    value:   ValueExpr::compile(expr)?,
                    low:     ValueExpr::compile(low)?,
                    high:    ValueExpr::compile(high)?,
                    negated: *negated,
                },
            },
            Expr::InList {
                expr,
                list,
                negated,
            } => {
                if list.len() > MAX_IN_LIST_VALUES {
                    return Err(RowFilterError::InvalidOperation(format!(
                        "IN list contains {} values, max supported is {}",
                        list.len(),
                        MAX_IN_LIST_VALUES
                    )));
                }
                CompiledPredicate {
                    complexity: list.len() + 1,
                    predicate:  Predicate::InList {
                        value:   ValueExpr::compile(expr)?,
                        list:    list
                            .iter()
                            .map(ValueExpr::compile)
                            .collect::<RowFilterResult<Vec<_>>>()?
                            .into_boxed_slice(),
                        negated: *negated,
                    },
                }
            },
            Expr::IsNull(expr) => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::IsNull {
                    value:   ValueExpr::compile(expr)?,
                    negated: false,
                },
            },
            Expr::IsNotNull(expr) => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::IsNull {
                    value:   ValueExpr::compile(expr)?,
                    negated: true,
                },
            },
            Expr::IsTrue(expr) => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::IsTrue {
                    value:   ValueExpr::compile(expr)?,
                    negated: false,
                },
            },
            Expr::IsNotTrue(expr) => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::IsTrue {
                    value:   ValueExpr::compile(expr)?,
                    negated: true,
                },
            },
            Expr::IsFalse(expr) => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::IsFalse {
                    value:   ValueExpr::compile(expr)?,
                    negated: false,
                },
            },
            Expr::IsNotFalse(expr) => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::IsFalse {
                    value:   ValueExpr::compile(expr)?,
                    negated: true,
                },
            },
            Expr::Like {
                negated,
                any,
                expr,
                pattern,
                escape_char,
            } => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::Like(LikePredicate::compile(
                    expr,
                    pattern,
                    *negated,
                    *any,
                    escape_char.as_ref().map(|v| &**v),
                    false,
                )?),
            },
            Expr::ILike {
                negated,
                any,
                expr,
                pattern,
                escape_char,
            } => CompiledPredicate {
                complexity: 1,
                predicate:  Predicate::Like(LikePredicate::compile(
                    expr,
                    pattern,
                    *negated,
                    *any,
                    escape_char.as_ref().map(|v| &**v),
                    true,
                )?),
            },
            Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Value(_) => {
                CompiledPredicate {
                    complexity: 1,
                    predicate:  Predicate::Bool(ValueExpr::compile(expr)?),
                }
            },
            Expr::InSubquery { .. }
            | Expr::AnyOp { .. }
            | Expr::AllOp { .. }
            | Expr::Exists { .. }
            | Expr::Subquery(_) => return Err(RowFilterError::UnsupportedSubquery),
            _ => return Err(RowFilterError::UnsupportedExpression(format!("{:?}", expr))),
        };

        Ok(compiled)
    }

    fn evaluate(&self, row_data: &Row) -> RowFilterResult<bool> {
        match self {
            Predicate::Bool(expr) => coerce_to_bool(expr.evaluate(row_data)?.as_ref()),
            Predicate::Not(inner) => Ok(!inner.evaluate(row_data)?),
            Predicate::And(left, right) => {
                if !left.evaluate(row_data)? {
                    return Ok(false);
                }
                right.evaluate(row_data)
            },
            Predicate::Or(left, right) => {
                if left.evaluate(row_data)? {
                    return Ok(true);
                }
                right.evaluate(row_data)
            },
            Predicate::Comparison {
                left,
                operator,
                right,
            } => {
                let left = left.evaluate(row_data)?;
                let right = right.evaluate(row_data)?;
                compare_scalars(left.as_ref(), *operator, right.as_ref())
            },
            Predicate::Between {
                value,
                low,
                high,
                negated,
            } => {
                let value = value.evaluate(row_data)?;
                let low = low.evaluate(row_data)?;
                let high = high.evaluate(row_data)?;
                let matched = compare_scalars(value.as_ref(), ComparisonOp::GtEq, low.as_ref())?
                    && compare_scalars(value.as_ref(), ComparisonOp::LtEq, high.as_ref())?;
                Ok(if *negated { !matched } else { matched })
            },
            Predicate::InList {
                value,
                list,
                negated,
            } => {
                let value = value.evaluate(row_data)?;
                if value.is_null() {
                    return Ok(false);
                }
                let mut matched = false;
                for item in list.iter() {
                    let item = item.evaluate(row_data)?;
                    if compare_scalars(value.as_ref(), ComparisonOp::Eq, item.as_ref())? {
                        matched = true;
                        break;
                    }
                }
                Ok(if *negated { !matched } else { matched })
            },
            Predicate::IsNull { value, negated } => {
                let value = value.evaluate(row_data)?;
                let is_null = value.is_null();
                Ok(if *negated { !is_null } else { is_null })
            },
            Predicate::IsTrue { value, negated } => {
                let value = value.evaluate(row_data)?;
                let is_true = matches!(value.as_ref(), ScalarValue::Boolean(Some(true)));
                Ok(if *negated { !is_true } else { is_true })
            },
            Predicate::IsFalse { value, negated } => {
                let value = value.evaluate(row_data)?;
                let is_false = matches!(value.as_ref(), ScalarValue::Boolean(Some(false)));
                Ok(if *negated { !is_false } else { is_false })
            },
            Predicate::Like(like) => like.evaluate(row_data),
        }
    }
}

fn coerce_to_bool(value: &ScalarValue) -> RowFilterResult<bool> {
    if value.is_null() {
        return Ok(false);
    }

    match value {
        ScalarValue::Boolean(Some(value)) => Ok(*value),
        _ => Err(RowFilterError::InvalidOperation(format!(
            "cannot coerce {:?} to boolean",
            value
        ))),
    }
}
