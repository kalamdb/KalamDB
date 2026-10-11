//! The only Rust ↔ V8 value translation.
//!
//! Strings are UTF-8 in Rust and Latin-1 or UTF-16 inside V8, so each string
//! is copied once with `v8::String::new`. External V8 strings only avoid that
//! copy for buffers that are already Latin-1 or UTF-16. Byte strings move
//! once into an `ArrayBuffer` backing store and surface as `Uint8Array`.
//! Objects cannot alias Arrow structs, so they are built field by field.
//! JSON SQL values are `JSON.parse`d from the UTF-8 already stored on the
//! routine value.

use std::sync::Arc;

use arrow::{
    array::{Array, StructArray},
    datatypes::{DataType, Field, TimeUnit},
};
use datafusion_common::ScalarValue;
use v8::{self, Local, PinScope};

use crate::{
    abi::conversion_budget::ConversionBudget,
    error::{FunctionsError, Result},
    value::RoutineValue,
};

pub fn routine_to_v8<'s>(
    scope: &PinScope<'s, '_>,
    value: &RoutineValue,
) -> Result<Local<'s, v8::Value>> {
    if value.json_sql {
        return json_text_to_v8(scope, &value.value);
    }
    scalar_to_v8(scope, &value.value)
}

pub(crate) fn string_to_v8<'s>(
    scope: &PinScope<'s, '_>,
    text: &str,
) -> Result<Local<'s, v8::String>> {
    v8::String::new(scope, text)
        .ok_or_else(|| FunctionsError::Invalid("string exceeds the v8 limit".into()))
}

pub(crate) fn string_from_v8(
    scope: &PinScope<'_, '_>,
    value: Local<'_, v8::Value>,
) -> Result<String> {
    let text = value
        .to_string(scope)
        .ok_or_else(|| FunctionsError::Invalid("string conversion failed".into()))?;
    Ok(text.to_rust_string_lossy(scope))
}

fn string_from_v8_budgeted<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    budget: &mut ConversionBudget,
) -> Result<String> {
    let text = value
        .to_string(scope)
        .ok_or_else(|| FunctionsError::Invalid("string conversion failed".into()))?;
    budget.string(text)?;
    Ok(text.to_rust_string_lossy(scope))
}

pub(crate) fn object_set(
    scope: &PinScope<'_, '_>,
    object: Local<'_, v8::Object>,
    key: &str,
    value: Local<'_, v8::Value>,
) -> Result<()> {
    let key = string_to_v8(scope, key)?;
    object
        .set(scope, key.into(), value)
        .ok_or_else(|| FunctionsError::Invalid("failed to set object field".into()))?;
    Ok(())
}

pub(crate) fn json_text_to_v8<'s>(
    scope: &PinScope<'s, '_>,
    value: &ScalarValue,
) -> Result<Local<'s, v8::Value>> {
    let text = match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => text.as_str(),
        ScalarValue::Utf8(None)
        | ScalarValue::LargeUtf8(None)
        | ScalarValue::Utf8View(None)
        | ScalarValue::Null => {
            return Ok(v8::null(scope).into());
        },
        other => {
            return Err(FunctionsError::Invalid(format!(
                "json sql value must be utf8, got {other:?}"
            )));
        },
    };
    let source = string_to_v8(scope, text)?;
    v8::json::parse(scope, source)
        .ok_or_else(|| FunctionsError::Invalid("invalid json sql value".into()))
}

pub fn v8_to_routine<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    template: &RoutineValue,
) -> Result<RoutineValue> {
    if template.json_sql {
        let parsed = v8_to_json_sql(scope, value)?;
        return Ok(RoutineValue {
            type_id:       template.type_id.clone(),
            value:         parsed,
            json_sql:      true,
            transfer:      None,
            contract_hash: None,
        });
    }
    let mut budget = ConversionBudget::new(scope);
    let scalar = v8_to_scalar(scope, value, &template.value.data_type(), &mut budget, 0)?;
    Ok(RoutineValue {
        type_id:       template.type_id.clone(),
        value:         scalar,
        json_sql:      false,
        transfer:      None,
        contract_hash: None,
    })
}

fn v8_to_json_sql<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
) -> Result<ScalarValue> {
    if value.is_null_or_undefined() {
        return Ok(ScalarValue::Utf8(None));
    }
    let text = v8::json::stringify(scope, value)
        .ok_or_else(|| FunctionsError::Invalid("failed to stringify json sql value".into()))?;
    let mut budget = ConversionBudget::new(scope);
    budget.string(text)?;
    Ok(ScalarValue::Utf8(Some(text.to_rust_string_lossy(scope))))
}

fn scalar_to_v8<'s>(scope: &PinScope<'s, '_>, value: &ScalarValue) -> Result<Local<'s, v8::Value>> {
    Ok(match value {
        ScalarValue::Null => v8::null(scope).into(),
        ScalarValue::Boolean(Some(flag)) => v8::Boolean::new(scope, *flag).into(),
        ScalarValue::Boolean(None) => v8::null(scope).into(),
        ScalarValue::Int8(Some(n)) => v8::Number::new(scope, *n as f64).into(),
        ScalarValue::Int16(Some(n)) => v8::Number::new(scope, *n as f64).into(),
        ScalarValue::Int32(Some(n)) => v8::Integer::new(scope, *n).into(),
        ScalarValue::Int64(Some(n)) => i64_to_v8(scope, *n),
        ScalarValue::UInt8(Some(n)) => v8::Number::new(scope, *n as f64).into(),
        ScalarValue::UInt16(Some(n)) => v8::Number::new(scope, *n as f64).into(),
        ScalarValue::UInt32(Some(n)) => v8::Number::new(scope, *n as f64).into(),
        ScalarValue::UInt64(Some(n)) => u64_to_v8(scope, *n),
        ScalarValue::Float32(Some(n)) => v8::Number::new(scope, *n as f64).into(),
        ScalarValue::Float64(Some(n)) => v8::Number::new(scope, *n).into(),
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => string_to_v8(scope, text)?.into(),
        ScalarValue::Binary(Some(bytes))
        | ScalarValue::LargeBinary(Some(bytes))
        | ScalarValue::BinaryView(Some(bytes)) => bytes_to_v8(scope, bytes)?,
        ScalarValue::FixedSizeBinary(_, Some(bytes)) if bytes.len() == 16 => {
            string_to_v8(scope, &uuid_text(bytes))?.into()
        },
        ScalarValue::FixedSizeBinary(_, Some(bytes)) => bytes_to_v8(scope, bytes)?,
        ScalarValue::Decimal128(Some(value), _precision, scale) => {
            string_to_v8(scope, &decimal128_text(*value, *scale))?.into()
        },
        ScalarValue::Decimal128(None, _, _)
        | ScalarValue::Binary(None)
        | ScalarValue::LargeBinary(None)
        | ScalarValue::BinaryView(None)
        | ScalarValue::FixedSizeBinary(_, None) => v8::null(scope).into(),
        ScalarValue::Utf8(None)
        | ScalarValue::LargeUtf8(None)
        | ScalarValue::Utf8View(None)
        | ScalarValue::Int8(None)
        | ScalarValue::Int16(None)
        | ScalarValue::Int32(None)
        | ScalarValue::Int64(None)
        | ScalarValue::UInt8(None)
        | ScalarValue::UInt16(None)
        | ScalarValue::UInt32(None)
        | ScalarValue::UInt64(None)
        | ScalarValue::Float32(None)
        | ScalarValue::Float64(None) => v8::null(scope).into(),
        ScalarValue::TimestampSecond(Some(ts), _) => i64_to_v8(scope, ts.saturating_mul(1000)),
        ScalarValue::TimestampMillisecond(Some(ts), _) => i64_to_v8(scope, *ts),
        ScalarValue::TimestampMicrosecond(Some(us), _) => i64_to_v8(scope, us.div_euclid(1000)),
        ScalarValue::TimestampNanosecond(Some(ns), _) => i64_to_v8(scope, ns.div_euclid(1_000_000)),
        ScalarValue::Date32(Some(days)) => i64_to_v8(scope, i64::from(*days) * 86_400_000),
        ScalarValue::Date64(Some(ms)) => i64_to_v8(scope, *ms),
        ScalarValue::TimestampSecond(None, _)
        | ScalarValue::TimestampMillisecond(None, _)
        | ScalarValue::TimestampMicrosecond(None, _)
        | ScalarValue::TimestampNanosecond(None, _)
        | ScalarValue::Date32(None)
        | ScalarValue::Date64(None) => v8::null(scope).into(),
        ScalarValue::Struct(array) => struct_to_v8(scope, array)?,
        ScalarValue::List(array) => list_to_v8(scope, array.as_ref())?,
        ScalarValue::LargeList(array) => list_to_v8(scope, array.as_ref())?,
        ScalarValue::FixedSizeList(array) => list_to_v8(scope, array.as_ref())?,
        other => {
            return Err(FunctionsError::Invalid(format!("unsupported function value: {other:?}")));
        },
    })
}

fn i64_to_v8<'s>(scope: &PinScope<'s, '_>, value: i64) -> Local<'s, v8::Value> {
    if value.unsigned_abs() <= (1u64 << 53) {
        v8::Number::new(scope, value as f64).into()
    } else {
        v8::BigInt::new_from_i64(scope, value).into()
    }
}

fn u64_to_v8<'s>(scope: &PinScope<'s, '_>, value: u64) -> Local<'s, v8::Value> {
    if value <= (1u64 << 53) {
        v8::Number::new(scope, value as f64).into()
    } else {
        v8::BigInt::new_from_u64(scope, value).into()
    }
}

fn bytes_to_v8<'s>(scope: &PinScope<'s, '_>, bytes: &[u8]) -> Result<Local<'s, v8::Value>> {
    let backing = v8::ArrayBuffer::new_backing_store_from_vec(bytes.to_vec()).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing);
    let view = v8::Uint8Array::new(scope, buffer, 0, bytes.len())
        .ok_or_else(|| FunctionsError::Invalid("byte value exceeds the v8 limit".into()))?;
    Ok(view.into())
}

fn bytes_from_v8<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    budget: &mut ConversionBudget,
) -> Result<Vec<u8>> {
    if let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) {
        return copy_uint8_array(view, budget);
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())
            .ok_or_else(|| FunctionsError::Invalid("byte value exceeds the v8 limit".into()))?;
        return copy_uint8_array(view, budget);
    }
    Err(FunctionsError::Invalid("expected Uint8Array or ArrayBuffer".into()))
}

fn copy_uint8_array(
    view: v8::Local<'_, v8::Uint8Array>,
    budget: &mut ConversionBudget,
) -> Result<Vec<u8>> {
    let len = view.byte_length();
    budget.bytes(len)?;
    let mut bytes = vec![0u8; len];
    let copied = view.copy_contents(&mut bytes);
    bytes.truncate(copied);
    Ok(bytes)
}

fn uuid_text(bytes: &[u8]) -> String {
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn decimal128_text(value: i128, scale: i8) -> String {
    let scale = u32::try_from(scale.max(0)).unwrap_or(0);
    if scale == 0 {
        return value.to_string();
    }
    let negative = value < 0;
    let value = value.checked_abs().unwrap_or(i128::MAX);
    let Some(divisor) = 10i128.checked_pow(scale) else {
        return value.to_string();
    };
    let integer = value / divisor;
    let fraction = value % divisor;
    let body = format!("{integer}.{fraction:0width$}", width = scale as usize);
    if negative {
        format!("-{body}")
    } else {
        body
    }
}

fn struct_to_v8<'s>(scope: &PinScope<'s, '_>, array: &StructArray) -> Result<Local<'s, v8::Value>> {
    if array.len() == 0 || array.is_null(0) {
        return Ok(v8::null(scope).into());
    }
    let object = v8::Object::new(scope);
    for (index, field) in array.fields().iter().enumerate() {
        let column = array.column(index);
        let scalar = ScalarValue::try_from_array(column, 0).map_err(|error| {
            FunctionsError::Invalid(format!("struct field '{}': {error}", field.name()))
        })?;
        let js_value = scalar_to_v8(scope, &scalar)?;
        object_set(scope, object, field.name(), js_value)?;
    }
    Ok(object.into())
}

fn list_to_v8<'s>(scope: &PinScope<'s, '_>, array: &dyn Array) -> Result<Local<'s, v8::Value>> {
    let Some(values) = list_values(array) else {
        return Ok(v8::null(scope).into());
    };
    let js_array = v8::Array::new(scope, values.len() as i32);
    for index in 0..values.len() {
        let scalar = ScalarValue::try_from_array(values.as_ref(), index)
            .map_err(|error| FunctionsError::Invalid(format!("list element {index}: {error}")))?;
        let js_value = scalar_to_v8(scope, &scalar)?;
        js_array
            .set_index(scope, index as u32, js_value)
            .ok_or_else(|| FunctionsError::Invalid("failed to set list element".into()))?;
    }
    Ok(js_array.into())
}

fn list_values(array: &dyn Array) -> Option<arrow::array::ArrayRef> {
    if let Some(list) = array.as_any().downcast_ref::<arrow::array::ListArray>() {
        if list.len() == 0 || list.is_null(0) {
            return None;
        }
        return Some(list.value(0));
    }
    if let Some(list) = array.as_any().downcast_ref::<arrow::array::LargeListArray>() {
        if list.len() == 0 || list.is_null(0) {
            return None;
        }
        return Some(list.value(0));
    }
    if let Some(list) = array.as_any().downcast_ref::<arrow::array::FixedSizeListArray>() {
        if list.len() == 0 || list.is_null(0) {
            return None;
        }
        return Some(list.value(0));
    }
    None
}

fn v8_to_scalar<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    data_type: &DataType,
    budget: &mut ConversionBudget,
    depth: usize,
) -> Result<ScalarValue> {
    budget.enter(scope, depth)?;
    if value.is_null_or_undefined() {
        return ScalarValue::try_from(data_type)
            .map_err(|error| FunctionsError::Invalid(error.to_string()));
    }
    match data_type {
        DataType::Boolean => Ok(ScalarValue::Boolean(Some(value.boolean_value(scope)))),
        DataType::Int8 => Ok(ScalarValue::Int8(Some(value.int32_value(scope).unwrap_or(0) as i8))),
        DataType::Int16 => {
            Ok(ScalarValue::Int16(Some(value.int32_value(scope).unwrap_or(0) as i16)))
        },
        DataType::Int32 => Ok(ScalarValue::Int32(Some(value.int32_value(scope).unwrap_or(0)))),
        DataType::Int64 => Ok(ScalarValue::Int64(Some(js_to_i64(scope, value)))),
        DataType::UInt8 => {
            Ok(ScalarValue::UInt8(Some(value.uint32_value(scope).unwrap_or(0) as u8)))
        },
        DataType::UInt16 => {
            Ok(ScalarValue::UInt16(Some(value.uint32_value(scope).unwrap_or(0) as u16)))
        },
        DataType::UInt32 => Ok(ScalarValue::UInt32(Some(value.uint32_value(scope).unwrap_or(0)))),
        DataType::UInt64 => Ok(ScalarValue::UInt64(Some(js_to_u64(scope, value)))),
        DataType::Float32 => {
            Ok(ScalarValue::Float32(Some(value.number_value(scope).unwrap_or(0.0) as f32)))
        },
        DataType::Float64 => {
            Ok(ScalarValue::Float64(Some(value.number_value(scope).unwrap_or(0.0))))
        },
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            Ok(ScalarValue::Utf8(Some(string_from_v8_budgeted(scope, value, budget)?)))
        },
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => {
            Ok(ScalarValue::Binary(Some(bytes_from_v8(scope, value, budget)?)))
        },
        DataType::FixedSizeBinary(16) => fixed_size_binary_from_v8(scope, value, 16, budget),
        DataType::FixedSizeBinary(size) => {
            let bytes = bytes_from_v8(scope, value, budget)?;
            if bytes.len() != *size as usize {
                return Err(FunctionsError::Invalid(format!(
                    "expected {size} bytes, got {}",
                    bytes.len()
                )));
            }
            Ok(ScalarValue::FixedSizeBinary(*size, Some(bytes)))
        },
        DataType::Decimal128(precision, scale) => {
            let text = string_from_v8_budgeted(scope, value, budget)?;
            Ok(parse_decimal128(&text, *precision, *scale)?)
        },
        DataType::Struct(fields) => v8_object_to_struct(scope, value, fields, budget, depth),
        DataType::List(field) => v8_array_to_list(scope, value, field, budget, depth),
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            Ok(ScalarValue::TimestampMicrosecond(Some(js_to_i64(scope, value)), tz.clone()))
        },
        DataType::Null => infer_value(scope, value, budget, depth),
        other => Err(FunctionsError::Invalid(format!("unsupported function return type: {other}"))),
    }
}

pub fn infer_v8_value<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
) -> Result<ScalarValue> {
    infer_value(scope, value, &mut ConversionBudget::new(scope), 0)
}

pub(crate) fn infer_value<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    budget: &mut ConversionBudget,
    depth: usize,
) -> Result<ScalarValue> {
    budget.enter(scope, depth)?;
    if value.is_null_or_undefined() {
        return Ok(ScalarValue::Null);
    }
    if value.is_boolean() {
        return Ok(ScalarValue::Boolean(Some(value.boolean_value(scope))));
    }
    if value.is_int32() {
        return Ok(ScalarValue::Int32(Some(value.int32_value(scope).unwrap_or(0))));
    }
    if value.is_number() {
        let number = value.number_value(scope).unwrap_or(0.0);
        if number.fract() == 0.0 && number.abs() <= i64::MAX as f64 {
            return Ok(ScalarValue::Int64(Some(number as i64)));
        }
        return Ok(ScalarValue::Float64(Some(number)));
    }
    if value.is_string() {
        return Ok(ScalarValue::Utf8(Some(string_from_v8_budgeted(scope, value, budget)?)));
    }
    if value.is_uint8_array() || value.is_array_buffer() {
        return Ok(ScalarValue::Binary(Some(bytes_from_v8(scope, value, budget)?)));
    }
    if value.is_array() {
        let array = v8::Local::<v8::Array>::try_from(value)
            .map_err(|_| FunctionsError::Invalid("expected array".to_string()))?;
        let len = array.length();
        budget.items(len as usize)?;
        let mut items = Vec::with_capacity(len as usize);
        for index in 0..len {
            let element = array
                .get_index(scope, index)
                .ok_or_else(|| FunctionsError::Javascript("array accessor failed".into()))?;
            items.push(infer_value(scope, element, budget, depth + 1)?);
        }
        let item_type = items
            .iter()
            .find(|item| !item.is_null())
            .map(|item| item.data_type())
            .unwrap_or(DataType::Utf8);
        for item in &mut items {
            if item.is_null() {
                *item = ScalarValue::try_from(&item_type)
                    .map_err(|error| FunctionsError::Invalid(error.to_string()))?;
            } else if item.data_type() != item_type {
                return Err(FunctionsError::Invalid(
                    "array elements must have a consistent type".into(),
                ));
            }
        }
        return Ok(ScalarValue::List(ScalarValue::new_list(&items, &item_type, true)));
    }
    if value.is_object() {
        return infer_v8_object(scope, value, budget, depth);
    }
    Ok(ScalarValue::Utf8(Some(string_from_v8_budgeted(scope, value, budget)?)))
}

fn infer_v8_object<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    budget: &mut ConversionBudget,
    depth: usize,
) -> Result<ScalarValue> {
    let object = value
        .to_object(scope)
        .ok_or_else(|| FunctionsError::Invalid("expected object".to_string()))?;
    let names = object
        .get_own_property_names(scope, v8::GetPropertyNamesArgsBuilder::new().build())
        .ok_or_else(|| FunctionsError::Invalid("failed to list object keys".to_string()))?;
    budget.items(names.length() as usize)?;
    let mut columns: Vec<(Arc<Field>, arrow::array::ArrayRef)> =
        Vec::with_capacity(names.length() as usize);
    for index in 0..names.length() {
        let key_value = names
            .get_index(scope, index)
            .ok_or_else(|| FunctionsError::Invalid("missing object key".to_string()))?;
        let key = string_from_v8_budgeted(scope, key_value, budget)?;
        let property = object
            .get(scope, key_value)
            .ok_or_else(|| FunctionsError::Javascript("object accessor failed".into()))?;
        let scalar = infer_value(scope, property, budget, depth + 1)?;
        let field = Arc::new(Field::new(&key, scalar.data_type(), true));
        let array = scalar
            .to_array()
            .map_err(|error| FunctionsError::Invalid(format!("object field '{key}': {error}")))?;
        columns.push((field, array));
    }
    Ok(ScalarValue::Struct(Arc::new(StructArray::from(columns))))
}

fn js_to_i64(scope: &PinScope<'_, '_>, value: Local<'_, v8::Value>) -> i64 {
    if value.is_big_int() {
        return value.to_big_int(scope).map(|n| n.i64_value().0).unwrap_or(0);
    }
    value.number_value(scope).unwrap_or(0.0) as i64
}

fn js_to_u64(scope: &PinScope<'_, '_>, value: Local<'_, v8::Value>) -> u64 {
    if value.is_big_int() {
        return value.to_big_int(scope).map(|n| n.u64_value().0).unwrap_or(0);
    }
    value.number_value(scope).unwrap_or(0.0) as u64
}

fn fixed_size_binary_from_v8<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    size: i32,
    budget: &mut ConversionBudget,
) -> Result<ScalarValue> {
    if size == 16 && value.is_string() {
        let text = string_from_v8_budgeted(scope, value, budget)?;
        return Ok(ScalarValue::FixedSizeBinary(16, Some(parse_uuid(&text)?.to_vec())));
    }
    let bytes = bytes_from_v8(scope, value, budget)?;
    if bytes.len() != usize::try_from(size).unwrap_or(usize::MAX) {
        return Err(FunctionsError::Invalid(format!("expected {size} bytes, got {}", bytes.len())));
    }
    Ok(ScalarValue::FixedSizeBinary(size, Some(bytes)))
}

fn parse_uuid(text: &str) -> Result<[u8; 16]> {
    let hex_text: String = text.chars().filter(|ch| *ch != '-').collect();
    let bytes = hex::decode(hex_text)
        .map_err(|error| FunctionsError::Invalid(format!("invalid uuid: {error}")))?;
    bytes.try_into().map_err(|_| FunctionsError::Invalid("invalid uuid".into()))
}

fn parse_decimal128(text: &str, precision: u8, scale: i8) -> Result<ScalarValue> {
    let negative = text.starts_with('-');
    let text = text.trim_start_matches(['+', '-']);
    let (integer, fraction) = text.split_once('.').unwrap_or((text, ""));
    let integer = if integer.is_empty() { "0" } else { integer };
    let scale_digits = usize::try_from(scale.max(0)).unwrap_or(0);
    if !integer.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
        || fraction.len() > scale_digits
    {
        return Err(FunctionsError::Invalid(format!("invalid decimal '{text}'")));
    }
    let mut digits = String::with_capacity(integer.len() + scale_digits);
    digits.push_str(integer);
    digits.push_str(fraction);
    for _ in fraction.len()..scale_digits {
        digits.push('0');
    }
    let mut value = digits
        .parse::<i128>()
        .map_err(|error| FunctionsError::Invalid(format!("invalid decimal '{text}': {error}")))?;
    if negative {
        value = -value;
    }
    Ok(ScalarValue::Decimal128(Some(value), precision, scale))
}

fn v8_object_to_struct<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    fields: &arrow::datatypes::Fields,
    budget: &mut ConversionBudget,
    depth: usize,
) -> Result<ScalarValue> {
    let object = value
        .to_object(scope)
        .ok_or_else(|| FunctionsError::Invalid("expected object for struct".to_string()))?;
    budget.items(fields.len())?;
    let mut columns: Vec<(Arc<Field>, arrow::array::ArrayRef)> = Vec::with_capacity(fields.len());
    for field in fields.iter() {
        let key = string_to_v8(scope, field.name())?;
        let property = object
            .get(scope, key.into())
            .ok_or_else(|| FunctionsError::Javascript("object accessor failed".into()))?;
        let scalar = v8_to_scalar(scope, property, field.data_type(), budget, depth + 1)?;
        let array = scalar.to_array().map_err(|error| {
            FunctionsError::Invalid(format!("struct field '{}': {error}", field.name()))
        })?;
        columns.push((Arc::clone(field), array));
    }
    Ok(ScalarValue::Struct(Arc::new(StructArray::from(columns))))
}

fn v8_array_to_list<'s>(
    scope: &PinScope<'s, '_>,
    value: Local<'s, v8::Value>,
    field: &Arc<Field>,
    budget: &mut ConversionBudget,
    depth: usize,
) -> Result<ScalarValue> {
    let array = v8::Local::<v8::Array>::try_from(value)
        .map_err(|_| FunctionsError::Invalid("expected array for list".to_string()))?;
    let len = array.length();
    budget.items(len as usize)?;
    let mut items = Vec::with_capacity(len as usize);
    for index in 0..len {
        let element = array
            .get_index(scope, index)
            .ok_or_else(|| FunctionsError::Javascript("array accessor failed".into()))?;
        items.push(v8_to_scalar(scope, element, field.data_type(), budget, depth + 1)?);
    }
    Ok(ScalarValue::List(ScalarValue::new_list(&items, field.data_type(), true)))
}

#[cfg(test)]
mod tests {
    use kalamdb_serialization::encode_function_value;
    use serde_json::json;

    use super::*;

    #[test]
    fn transfer_buffer_roundtrips_through_encoder() {
        let json = json!({"n": 7});
        let bytes = encode_function_value("c1", &json).unwrap();
        let decoded = kalamdb_serialization::decode_function_value(&bytes, "c1").unwrap();
        assert_eq!(decoded, json);
        let value = RoutineValue::new(ScalarValue::Int32(Some(7)))
            .with_transfer(bytes::Bytes::from(bytes), "c1");
        assert!(value.transfer.is_some());
        assert_eq!(value.contract_hash.as_deref(), Some("c1"));
    }

    #[test]
    fn js_boundary_json_keeps_safe_int64_as_number() {
        let encoded = kalamdb_commons::conversions::arrow_json_conversion::scalar_value_to_js_json(
            &ScalarValue::Int64(Some(41)),
        )
        .expect("js json");
        assert_eq!(encoded.0, json!(41));
        let bytes = encode_function_value("", &encoded.0).unwrap();
        let decoded = kalamdb_serialization::decode_function_value(&bytes, "").unwrap();
        assert_eq!(decoded, json!(41));
    }

    #[test]
    fn uuid_and_decimal_text_match_the_json_boundary() {
        let uuid = super::uuid_text(&[
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ]);
        assert_eq!(uuid, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(super::decimal128_text(20075, 2), "200.75");
        assert_eq!(super::decimal128_text(-75, 2), "-0.75");
        assert_eq!(
            super::parse_decimal128("-0.75", 10, 2).unwrap(),
            ScalarValue::Decimal128(Some(-75), 10, 2)
        );
    }
}
