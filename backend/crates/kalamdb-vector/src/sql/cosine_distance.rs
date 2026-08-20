use std::sync::{Arc, OnceLock};

use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, FixedSizeListArray, Float32Array, Float64Array, Int32Array,
            Int64Array, LargeListArray, ListArray, UInt32Array, UInt64Array,
        },
        datatypes::DataType,
    },
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
    scalar::ScalarValue,
};
use kalamdb_commons::arrow_utils::{arrow_float32, ArrowDataType};

static NATIVE_COSINE_DISTANCE: OnceLock<Arc<ScalarUDF>> = OnceLock::new();

fn native_cosine_distance_udf() -> Arc<ScalarUDF> {
    NATIVE_COSINE_DISTANCE
        .get_or_init(|| datafusion::functions_nested::cosine_distance::cosine_distance_udf())
        .clone()
}

fn uses_json_query_vector_types(arg_types: &[DataType]) -> bool {
    matches!(arg_types.get(1), Some(DataType::Utf8 | DataType::LargeUtf8))
}

fn uses_json_query_vector(args: &[ColumnarValue]) -> bool {
    matches!(
        args.get(1),
        Some(ColumnarValue::Scalar(ScalarValue::Utf8(_) | ScalarValue::LargeUtf8(_)))
    )
}

fn json_query_vector_return_type(_arg_types: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
    Ok(arrow_float32())
}

fn json_query_vector_coerce_types(
    arg_types: &[ArrowDataType],
) -> DataFusionResult<Vec<ArrowDataType>> {
    if arg_types.len() != 2 {
        return Err(DataFusionError::Plan(
            "COSINE_DISTANCE() expects exactly 2 arguments".to_string(),
        ));
    }
    Ok(arg_types.to_vec())
}

/// Dispatches `cosine_distance` to DataFusion 55 for array/array inputs and to the
/// Kalam JSON-literal path when the query vector is a UTF-8 JSON string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CosineDistanceFunction;

impl CosineDistanceFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for CosineDistanceFunction {
    fn name(&self) -> &str {
        "cosine_distance"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| Signature::user_defined(Volatility::Immutable))
    }

    fn return_type(&self, arg_types: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        if uses_json_query_vector_types(arg_types) {
            json_query_vector_return_type(arg_types)
        } else {
            native_cosine_distance_udf().return_type(arg_types)
        }
    }

    fn coerce_types(&self, arg_types: &[ArrowDataType]) -> DataFusionResult<Vec<ArrowDataType>> {
        if uses_json_query_vector_types(arg_types) {
            json_query_vector_coerce_types(arg_types)
        } else {
            native_cosine_distance_udf().inner().coerce_types(arg_types)
        }
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if uses_json_query_vector(&args.args) {
            invoke_json_query_cosine_distance(&args.args)
        } else {
            native_cosine_distance_udf().invoke_with_args(args)
        }
    }
}

fn invoke_json_query_cosine_distance(args: &[ColumnarValue]) -> DataFusionResult<ColumnarValue> {
    if args.len() != 2 {
        return Err(DataFusionError::Plan(
            "COSINE_DISTANCE() expects exactly 2 arguments".to_string(),
        ));
    }

    let query_vector = parse_query_vector_arg(&args[1])?;
    if query_vector.is_empty() {
        return Err(DataFusionError::Plan(
            "COSINE_DISTANCE() query vector cannot be empty".to_string(),
        ));
    }

    match &args[0] {
        ColumnarValue::Array(array) => {
            let mut out: Vec<Option<f32>> = Vec::with_capacity(array.len());
            for idx in 0..array.len() {
                let maybe_vector = parse_vector_at_index(array.as_ref(), idx)?;
                let distance = if let Some(vector) = maybe_vector {
                    if vector.len() != query_vector.len() {
                        return Err(DataFusionError::Execution(format!(
                            "COSINE_DISTANCE() dimension mismatch: row has {}, query has {}",
                            vector.len(),
                            query_vector.len()
                        )));
                    }
                    Some(json_cosine_distance(&vector, &query_vector))
                } else {
                    None
                };
                out.push(distance);
            }

            Ok(ColumnarValue::Array(Arc::new(Float32Array::from(out)) as ArrayRef))
        },
        ColumnarValue::Scalar(value) => {
            let maybe_vector = parse_vector_from_scalar(value)?;
            let scalar_out = if let Some(vector) = maybe_vector {
                if vector.len() != query_vector.len() {
                    return Err(DataFusionError::Execution(format!(
                        "COSINE_DISTANCE() dimension mismatch: value has {}, query has {}",
                        vector.len(),
                        query_vector.len()
                    )));
                }
                ScalarValue::Float32(Some(json_cosine_distance(&vector, &query_vector)))
            } else {
                ScalarValue::Float32(None)
            };

            Ok(ColumnarValue::Scalar(scalar_out))
        },
    }
}

fn json_cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0;
    }

    let similarity = (dot / (norm_a.sqrt() * norm_b.sqrt())).clamp(-1.0, 1.0);
    1.0 - similarity
}

fn parse_query_vector_arg(value: &ColumnarValue) -> DataFusionResult<Vec<f32>> {
    match value {
        ColumnarValue::Scalar(scalar) => parse_vector_from_scalar(scalar)?.ok_or_else(|| {
            DataFusionError::Execution("COSINE_DISTANCE() query vector cannot be NULL".to_string())
        }),
        ColumnarValue::Array(array) => {
            if array.is_empty() {
                return Err(DataFusionError::Execution(
                    "COSINE_DISTANCE() query vector cannot be empty".to_string(),
                ));
            }
            parse_vector_at_index(array.as_ref(), 0)?.ok_or_else(|| {
                DataFusionError::Execution(
                    "COSINE_DISTANCE() query vector cannot be NULL".to_string(),
                )
            })
        },
    }
}

fn parse_vector_from_scalar(value: &ScalarValue) -> DataFusionResult<Option<Vec<f32>>> {
    match value {
        ScalarValue::Null => Ok(None),
        ScalarValue::Utf8(Some(json)) | ScalarValue::LargeUtf8(Some(json)) => {
            let parsed = serde_json::from_str::<Vec<f32>>(json).map_err(|e| {
                DataFusionError::Execution(format!(
                    "COSINE_DISTANCE() failed to parse JSON query vector: {}",
                    e
                ))
            })?;
            Ok(Some(parsed))
        },
        ScalarValue::FixedSizeList(array) => parse_vector_at_index(array.as_ref(), 0),
        ScalarValue::List(array) => parse_vector_at_index(array.as_ref(), 0),
        ScalarValue::LargeList(array) => parse_vector_at_index(array.as_ref(), 0),
        _ => Err(DataFusionError::Plan(format!(
            "COSINE_DISTANCE() unsupported scalar type for vector argument: {:?}",
            value.data_type()
        ))),
    }
}

fn parse_vector_at_index(array: &dyn Array, idx: usize) -> DataFusionResult<Option<Vec<f32>>> {
    match array.data_type() {
        DataType::FixedSizeList(_, _) => {
            let list = array.as_any().downcast_ref::<FixedSizeListArray>().ok_or_else(|| {
                DataFusionError::Execution(
                    "COSINE_DISTANCE() invalid fixed-size list vector argument".to_string(),
                )
            })?;
            if idx >= list.len() || list.is_null(idx) {
                return Ok(None);
            }
            let values = list.value(idx);
            if values.null_count() == values.len() {
                return Ok(None);
            }
            parse_numeric_array(values.as_ref()).map(Some)
        },
        DataType::List(_) => {
            let list = array.as_any().downcast_ref::<ListArray>().ok_or_else(|| {
                DataFusionError::Execution(
                    "COSINE_DISTANCE() invalid list vector argument".to_string(),
                )
            })?;
            if idx >= list.len() || list.is_null(idx) {
                return Ok(None);
            }
            let values = list.value(idx);
            if values.null_count() == values.len() {
                return Ok(None);
            }
            parse_numeric_array(values.as_ref()).map(Some)
        },
        DataType::LargeList(_) => {
            let list = array.as_any().downcast_ref::<LargeListArray>().ok_or_else(|| {
                DataFusionError::Execution(
                    "COSINE_DISTANCE() invalid large-list vector argument".to_string(),
                )
            })?;
            if idx >= list.len() || list.is_null(idx) {
                return Ok(None);
            }
            let values = list.value(idx);
            if values.null_count() == values.len() {
                return Ok(None);
            }
            parse_numeric_array(values.as_ref()).map(Some)
        },
        _ => Err(DataFusionError::Plan(format!(
            "COSINE_DISTANCE() vector argument must be a list, got {:?}",
            array.data_type()
        ))),
    }
}

fn parse_numeric_array(values: &dyn Array) -> DataFusionResult<Vec<f32>> {
    match values.data_type() {
        DataType::Float32 => {
            let array = values.as_any().downcast_ref::<Float32Array>().ok_or_else(|| {
                DataFusionError::Execution("COSINE_DISTANCE() invalid Float32 vector".to_string())
            })?;
            Ok((0..array.len())
                .map(|idx| {
                    if array.is_null(idx) {
                        0.0
                    } else {
                        array.value(idx)
                    }
                })
                .collect())
        },
        DataType::Float64 => {
            let array = values.as_any().downcast_ref::<Float64Array>().ok_or_else(|| {
                DataFusionError::Execution("COSINE_DISTANCE() invalid Float64 vector".to_string())
            })?;
            Ok((0..array.len())
                .map(|idx| {
                    if array.is_null(idx) {
                        0.0
                    } else {
                        array.value(idx) as f32
                    }
                })
                .collect())
        },
        DataType::Int64 => {
            let array = values.as_any().downcast_ref::<Int64Array>().ok_or_else(|| {
                DataFusionError::Execution("COSINE_DISTANCE() invalid Int64 vector".to_string())
            })?;
            Ok((0..array.len())
                .map(|idx| {
                    if array.is_null(idx) {
                        0.0
                    } else {
                        array.value(idx) as f32
                    }
                })
                .collect())
        },
        DataType::Int32 => {
            let array = values.as_any().downcast_ref::<Int32Array>().ok_or_else(|| {
                DataFusionError::Execution("COSINE_DISTANCE() invalid Int32 vector".to_string())
            })?;
            Ok((0..array.len())
                .map(|idx| {
                    if array.is_null(idx) {
                        0.0
                    } else {
                        array.value(idx) as f32
                    }
                })
                .collect())
        },
        DataType::UInt64 => {
            let array = values.as_any().downcast_ref::<UInt64Array>().ok_or_else(|| {
                DataFusionError::Execution("COSINE_DISTANCE() invalid UInt64 vector".to_string())
            })?;
            Ok((0..array.len())
                .map(|idx| {
                    if array.is_null(idx) {
                        0.0
                    } else {
                        array.value(idx) as f32
                    }
                })
                .collect())
        },
        DataType::UInt32 => {
            let array = values.as_any().downcast_ref::<UInt32Array>().ok_or_else(|| {
                DataFusionError::Execution("COSINE_DISTANCE() invalid UInt32 vector".to_string())
            })?;
            Ok((0..array.len())
                .map(|idx| {
                    if array.is_null(idx) {
                        0.0
                    } else {
                        array.value(idx) as f32
                    }
                })
                .collect())
        },
        _ => Err(DataFusionError::Plan(format!(
            "COSINE_DISTANCE() list must contain numeric values, got {:?}",
            values.data_type()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use datafusion::{arrow::datatypes::Field, logical_expr::ScalarUDF};

    use super::*;

    #[test]
    fn test_cosine_distance_function_creation() {
        let func_impl = CosineDistanceFunction::new();
        let func = ScalarUDF::new_from_impl(func_impl);
        assert_eq!(func.name(), "cosine_distance");
    }

    #[test]
    fn test_json_cosine_distance_math() {
        let same = json_cosine_distance(&[1.0, 0.0], &[1.0, 0.0]);
        let orthogonal = json_cosine_distance(&[1.0, 0.0], &[0.0, 1.0]);
        assert!((same - 0.0).abs() < 1e-6);
        assert!((orthogonal - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_query_vector_json() {
        let parsed = parse_vector_from_scalar(&ScalarValue::Utf8(Some("[0.1, 0.2]".to_string())))
            .expect("json parse should succeed")
            .expect("vector should exist");
        assert_eq!(parsed, vec![0.1, 0.2]);
    }

    #[test]
    fn test_json_query_vector_coerce_types_preserves_json_query_vector() {
        let func = CosineDistanceFunction::new();
        let row_vector =
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 3);
        let arg_types = vec![row_vector.clone(), DataType::Utf8];

        let coerced = func.coerce_types(&arg_types).expect("coercion should pass");

        assert_eq!(coerced, arg_types);
    }

    #[test]
    fn test_cosine_distance_dispatcher_uses_native_for_array_query_vector() {
        let func = CosineDistanceFunction::new();
        let list_field = Arc::new(Field::new("item", DataType::Float64, true));
        let arg_types = vec![
            DataType::List(list_field.clone()),
            DataType::List(list_field),
        ];

        assert!(!uses_json_query_vector_types(&arg_types));
        assert!(func.coerce_types(&arg_types).is_ok());
    }

    #[test]
    fn test_parse_vector_at_index_treats_all_null_fixed_size_list_as_null() {
        let values = Float32Array::from(vec![None, None, None]);
        let list = FixedSizeListArray::new(
            Arc::new(Field::new("item", DataType::Float32, true)),
            3,
            Arc::new(values),
            None,
        );

        let parsed = parse_vector_at_index(&list, 0).expect("parse should succeed");
        assert!(parsed.is_none());
    }
}
