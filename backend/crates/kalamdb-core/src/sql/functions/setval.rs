//! PostgreSQL `setval(sequence, value [, is_called])` for `serial_id` counters.
//!
//! Directus resets an explicit integer primary key with
//! `SELECT SETVAL(pg_get_serial_sequence(...), MAX(id))`. The two-argument form sets the last
//! value and the next generated id is one past it.

use datafusion::{
    arrow::array::{Array, Int32Array, Int64Array},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature},
    scalar::ScalarValue,
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

use super::scalar::{bool_at, cached_aliases, cached_signature, text_at, user_defined_volatile};
use crate::sql::serial_sequence::set_serial_sequence;

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SetvalFunction;

impl SetvalFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for SetvalFunction {
    fn name(&self) -> &str {
        "setval"
    }

    fn aliases(&self) -> &[String] {
        cached_aliases!("pg_catalog.setval")
    }

    fn signature(&self) -> &Signature {
        cached_signature!(user_defined_volatile())
    }

    fn coerce_types(&self, types: &[ArrowDataType]) -> DataFusionResult<Vec<ArrowDataType>> {
        match types.len() {
            2 => Ok(vec![arrow_utf8(), ArrowDataType::Int64]),
            3 => Ok(vec![arrow_utf8(), ArrowDataType::Int64, ArrowDataType::Boolean]),
            _ => Err(DataFusionError::Plan(
                "setval requires a sequence name and an integer value".into(),
            )),
        }
    }

    fn return_type(&self, _: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(ArrowDataType::Int64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        let scalar_inputs = args.args.iter().all(|arg| matches!(arg, ColumnarValue::Scalar(_)));
        if scalar_inputs {
            let value = apply_setval(&args, 0)?;
            return Ok(ColumnarValue::Scalar(ScalarValue::Int64(value)));
        }
        let mut values = Vec::with_capacity(args.number_rows);
        for row in 0..args.number_rows {
            values.push(apply_setval(&args, row)?);
        }
        Ok(ColumnarValue::Array(std::sync::Arc::new(Int64Array::from(values))))
    }
}

fn apply_setval(args: &ScalarFunctionArgs, row: usize) -> DataFusionResult<Option<i64>> {
    let name = text_at(&args.args[0], row)?;
    let value = i64_at(&args.args[1], row)?;
    let (Some(name), Some(value)) = (name, value) else {
        return Ok(None);
    };
    let is_called = if args.args.len() == 3 {
        bool_at(&args.args[2], row)?
    } else {
        true
    };
    set_serial_sequence(name, value, is_called)
        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
    Ok(Some(value))
}

fn i64_at(value: &ColumnarValue, row: usize) -> DataFusionResult<Option<i64>> {
    match value {
        ColumnarValue::Scalar(ScalarValue::Int64(value)) => Ok(*value),
        ColumnarValue::Scalar(ScalarValue::Int32(value)) => Ok(value.map(i64::from)),
        ColumnarValue::Scalar(ScalarValue::Null) => Ok(None),
        ColumnarValue::Array(array) if array.is_null(row) => Ok(None),
        ColumnarValue::Array(array) => {
            if let Some(values) = array.as_any().downcast_ref::<Int64Array>() {
                return Ok(Some(values.value(row)));
            }
            if let Some(values) = array.as_any().downcast_ref::<Int32Array>() {
                return Ok(Some(i64::from(values.value(row))));
            }
            Err(DataFusionError::Execution("setval expected an integer value".into()))
        },
        _ => Err(DataFusionError::Execution("setval expected an integer value".into())),
    }
}
