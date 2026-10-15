//! `format_type(type_oid, typemod)` for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, Int32Array, Int64Array, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::ColumnarValue,
    scalar::ScalarValue,
};
use kalamdb_commons::arrow_utils::ArrowDataType;
use kalamdb_views::pg_catalog::type_mapping::pg_format_type;

use super::scalar::{arity, scalar_udf, stable_exact};

scalar_udf! {
    pub struct FormatTypeFunction;
    name = "format_type";
    aliases = ["pg_catalog.format_type"];
    signature = stable_exact(&[
        &[ArrowDataType::Int64, ArrowDataType::Int64],
        &[ArrowDataType::Int32, ArrowDataType::Int32],
        &[ArrowDataType::Int64, ArrowDataType::Int32],
        &[ArrowDataType::Int32, ArrowDataType::Int64],
        &[ArrowDataType::Int64, ArrowDataType::Null],
        &[ArrowDataType::Int32, ArrowDataType::Null],
        &[ArrowDataType::Int64, ArrowDataType::UInt64],
        &[ArrowDataType::Int32, ArrowDataType::UInt64],
    ]);
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 2, "format_type(type_oid, typemod) requires two arguments")?;
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
        ColumnarValue::Scalar(ScalarValue::Null) => Ok(vec![0; row_count]),
        ColumnarValue::Scalar(ScalarValue::Int64(oid)) => Ok(vec![oid.unwrap_or(0); row_count]),
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
