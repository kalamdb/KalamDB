//! `format_type(type_oid, typemod)` for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, Int32Array, Int64Array, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, TypeSignature, Volatility},
    scalar::ScalarValue,
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};
use kalamdb_views::pg_catalog::type_mapping::pg_format_type;

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct FormatTypeFunction;

impl FormatTypeFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for FormatTypeFunction {
    fn name(&self) -> &str {
        "format_type"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| {
            Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![ArrowDataType::Int64, ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32, ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![ArrowDataType::Int64, ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32, ArrowDataType::Int64]),
                ],
                Volatility::Stable,
            )
        })
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.len() != 2 {
            return Err(DataFusionError::Plan(
                "format_type(type_oid, typemod) requires two arguments".to_string(),
            ));
        }

        let type_ids = type_oid_array(&args.args[0], args.number_rows)?;
        let mut builder = StringBuilder::with_capacity(type_ids.len(), 16);
        for type_id in type_ids {
            builder.append_value(pg_format_type(type_id));
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}

fn type_oid_array(value: &ColumnarValue, row_count: usize) -> DataFusionResult<Vec<i64>> {
    match value {
        ColumnarValue::Scalar(ScalarValue::Int64(oid)) => {
            Ok(vec![oid.unwrap_or(0); row_count])
        },
        ColumnarValue::Scalar(ScalarValue::Int32(oid)) => {
            Ok(vec![oid.unwrap_or(0) as i64; row_count])
        },
        ColumnarValue::Array(array) => {
            if let Some(array) = array.as_any().downcast_ref::<Int64Array>() {
                return Ok(array.values().iter().copied().collect());
            }
            if let Some(array) = array.as_any().downcast_ref::<Int32Array>() {
                return Ok(array.values().iter().map(|value| *value as i64).collect());
            }
            Err(DataFusionError::Plan(
                "format_type(type_oid, typemod) requires integer type_oid".to_string(),
            ))
        },
        _ => Err(DataFusionError::Plan(
            "format_type(type_oid, typemod) requires integer type_oid".to_string(),
        )),
    }
}
