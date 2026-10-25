use std::borrow::Cow;

use datafusion_common::ScalarValue;
use kalamdb_commons::models::rows::Row;
use sqlparser::ast::{Expr, Value};

use crate::{RowFilterError, RowFilterResult};

#[derive(Clone, Debug)]
pub(crate) enum ValueExpr {
    Column(ColumnRef),
    Literal(ScalarValue),
}

#[derive(Clone, Debug)]
pub(crate) struct ColumnRef {
    name: String,
    lower_name: String,
    upper_name: String,
}

impl ColumnRef {
    fn new(name: String) -> Self {
        let lower_name = name.to_lowercase();
        let upper_name = name.to_uppercase();
        Self {
            name,
            lower_name,
            upper_name,
        }
    }

    fn lookup<'a>(&'a self, row_data: &'a Row) -> Option<&'a ScalarValue> {
        row_data
            .get(&self.name)
            .or_else(|| row_data.get(&self.lower_name))
            .or_else(|| row_data.get(&self.upper_name))
    }
}

impl ValueExpr {
    pub(crate) fn compile(expr: &Expr) -> RowFilterResult<Self> {
        match expr {
            Expr::Nested(inner) => Self::compile(inner),
            Expr::Identifier(ident) => Ok(Self::Column(ColumnRef::new(ident.value.clone()))),
            Expr::CompoundIdentifier(parts) => {
                let Some(ident) = parts.last() else {
                    return Err(RowFilterError::InvalidOperation(
                        "empty compound identifier".to_string(),
                    ));
                };
                Ok(Self::Column(ColumnRef::new(ident.value.clone())))
            },
            Expr::Value(value) => scalar_from_value(&value.value).map(Self::Literal),
            _ => Err(RowFilterError::UnsupportedExpression(format!("{:?}", expr))),
        }
    }

    pub(crate) fn evaluate<'a>(
        &'a self,
        row_data: &'a Row,
    ) -> RowFilterResult<Cow<'a, ScalarValue>> {
        match self {
            Self::Column(column) => Ok(column
                .lookup(row_data)
                .map(Cow::Borrowed)
                .unwrap_or_else(|| Cow::Owned(ScalarValue::Null))),
            Self::Literal(value) => Ok(Cow::Borrowed(value)),
        }
    }

    pub(crate) fn literal_string(&self) -> Option<&str> {
        match self {
            Self::Literal(value) => scalar_as_str(value),
            Self::Column(_) => None,
        }
    }
}

pub(crate) fn scalar_as_str(value: &ScalarValue) -> Option<&str> {
    match value {
        ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => {
            Some(value.as_str())
        },
        _ => None,
    }
}

fn scalar_from_value(value: &Value) -> RowFilterResult<ScalarValue> {
    match value {
        Value::SingleQuotedString(value) | Value::DoubleQuotedString(value) => {
            Ok(ScalarValue::Utf8(Some(value.clone())))
        },
        Value::Number(value, _) => {
            if let Ok(parsed) = value.parse::<i64>() {
                Ok(ScalarValue::Int64(Some(parsed)))
            } else if let Ok(parsed) = value.parse::<f64>() {
                Ok(ScalarValue::Float64(Some(parsed)))
            } else {
                Err(RowFilterError::InvalidOperation(format!("invalid number: {}", value)))
            }
        },
        Value::Boolean(value) => Ok(ScalarValue::Boolean(Some(*value))),
        Value::Null => Ok(ScalarValue::Null),
        _ => Err(RowFilterError::UnsupportedExpression(format!("{:?}", value))),
    }
}
