use datafusion_common::ScalarValue;
use serde_json::{Number, Value};

use crate::models::datatypes::KalamDataType;

pub fn json_value_to_scalar_for_column(
    value: &Value,
    data_type: &KalamDataType,
) -> Result<ScalarValue, String> {
    if value.is_null() {
        return Ok(ScalarValue::Null);
    }

    let scalar = match data_type {
        KalamDataType::Boolean => ScalarValue::Boolean(Some(value.as_bool().unwrap_or(false))),
        KalamDataType::SmallInt => {
            let num = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .unwrap_or(0);
            ScalarValue::Int16(Some(num as i16))
        },
        KalamDataType::Int => {
            let num = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .unwrap_or(0);
            ScalarValue::Int32(Some(num as i32))
        },
        KalamDataType::BigInt => {
            let num = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .unwrap_or(0);
            ScalarValue::Int64(Some(num))
        },
        KalamDataType::Timestamp => {
            let num = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .unwrap_or(0);
            ScalarValue::TimestampMicrosecond(Some(num), None)
        },
        KalamDataType::DateTime => {
            let num = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .unwrap_or(0);
            ScalarValue::TimestampMicrosecond(Some(num), Some("UTC".into()))
        },
        KalamDataType::Time => {
            let num = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .unwrap_or(0);
            ScalarValue::Time64Microsecond(Some(num))
        },
        KalamDataType::Date => {
            let num = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .unwrap_or(0);
            ScalarValue::Date32(Some(num as i32))
        },
        KalamDataType::Double => ScalarValue::Float64(Some(value.as_f64().unwrap_or(0.0))),
        KalamDataType::Float => ScalarValue::Float32(Some(value.as_f64().unwrap_or(0.0) as f32)),
        KalamDataType::Text | KalamDataType::Uuid => {
            let text = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
            ScalarValue::Utf8(Some(text))
        },
        KalamDataType::Json | KalamDataType::File => match value {
            Value::String(s) => ScalarValue::Utf8(Some(s.clone())),
            Value::Object(_) | Value::Array(_) => {
                // Store JSON objects/arrays as Utf8 JSON strings so DataFusion
                // JSON functions (datafusion-functions-json) can operate on them
                // directly without a binary↔string conversion layer.
                let json_str = serde_json::to_string(value)
                    .map_err(|e| format!("json field encode failed: {e}"))?;
                ScalarValue::Utf8(Some(json_str))
            },
            _ => ScalarValue::Utf8(Some(value.to_string())),
        },
        KalamDataType::Bytes => match value {
            Value::String(s) => ScalarValue::Binary(Some(s.as_bytes().to_vec())),
            Value::Array(arr) => {
                let bytes: Vec<u8> = arr
                    .iter()
                    .filter_map(|v| v.as_u64().and_then(|u| u8::try_from(u).ok()))
                    .collect();
                ScalarValue::Binary(Some(bytes))
            },
            _ => ScalarValue::Binary(Some(value.to_string().into_bytes())),
        },
        KalamDataType::Decimal { precision, scale } => {
            let parsed = match value {
                Value::String(s) => parse_decimal_value_bounded(s, *precision, *scale as i8)?,
                Value::Number(n) => {
                    parse_decimal_value_bounded(&n.to_string(), *precision, *scale as i8)?
                },
                _ => 0,
            };
            ScalarValue::Decimal128(Some(parsed), *precision, *scale as i8)
        },
        KalamDataType::Embedding(_) => ScalarValue::Utf8(Some(value.to_string())),
    };

    Ok(scalar)
}

pub fn scalar_to_json_for_column(
    scalar: &ScalarValue,
    data_type: &KalamDataType,
) -> Result<Value, String> {
    if matches!(scalar, ScalarValue::Null) {
        return Ok(Value::Null);
    }

    let json = match data_type {
        KalamDataType::Boolean => Value::Bool(extract_bool(scalar).unwrap_or(false)),
        KalamDataType::SmallInt
        | KalamDataType::Int
        | KalamDataType::BigInt
        | KalamDataType::Date => {
            let value = extract_i64(scalar).unwrap_or(0);
            Value::Number(Number::from(value))
        },
        KalamDataType::Timestamp | KalamDataType::DateTime => {
            let value = extract_timestamp_micros(scalar).unwrap_or(0);
            Value::Number(Number::from(value))
        },
        KalamDataType::Time => {
            let value = extract_time_micros(scalar).unwrap_or(0);
            Value::Number(Number::from(value))
        },
        KalamDataType::Double | KalamDataType::Float => {
            let value = extract_f64(scalar).unwrap_or(0.0);
            Value::Number(
                Number::from_f64(value)
                    .ok_or_else(|| "invalid floating point value".to_string())?,
            )
        },
        KalamDataType::Text | KalamDataType::Uuid => {
            Value::String(extract_string(scalar).unwrap_or_default())
        },
        KalamDataType::Json | KalamDataType::File => match scalar {
            ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                // Try to parse as structured JSON (objects, arrays, etc.) so
                // that system models with Vec / struct fields deserialize
                // correctly.  Plain strings that happen to look like JSON text
                // (e.g. a string value `"hello"`) fall back to Value::String.
                serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone()))
            },
            ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) => Value::Null,
            _ => Value::String(extract_string(scalar).unwrap_or_default()),
        },
        KalamDataType::Bytes => match scalar {
            ScalarValue::Binary(Some(bytes)) | ScalarValue::LargeBinary(Some(bytes)) => {
                Value::Array(bytes.iter().map(|b| Value::Number(Number::from(*b))).collect())
            },
            ScalarValue::Binary(None) | ScalarValue::LargeBinary(None) => Value::Null,
            _ => Value::String(extract_string(scalar).unwrap_or_default()),
        },
        KalamDataType::Decimal { scale, .. } => match scalar {
            ScalarValue::Decimal128(Some(value), _, _) => {
                Value::String(format_decimal_value(*value, *scale))
            },
            ScalarValue::Decimal128(None, _, _) => Value::Null,
            _ => Value::String(extract_string(scalar).unwrap_or_default()),
        },
        KalamDataType::Embedding(_) => Value::String(extract_string(scalar).unwrap_or_default()),
    };

    Ok(json)
}

/// Parse a decimal string into an unscaled `i128` for `NUMERIC(precision, scale)`.
///
/// Rounds fractional digits half-away-from-zero, rejects `NaN` / `Infinity`,
/// and checks that the rounded magnitude fits within `precision` digits.
pub fn parse_decimal_value_bounded(
    value: &str,
    precision: u8,
    scale: i8,
) -> Result<i128, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("cannot parse empty string as numeric".to_string());
    }
    if trimmed.eq_ignore_ascii_case("nan")
        || trimmed.eq_ignore_ascii_case("infinity")
        || trimmed.eq_ignore_ascii_case("-infinity")
        || trimmed.eq_ignore_ascii_case("+infinity")
        || trimmed.eq_ignore_ascii_case("inf")
        || trimmed.eq_ignore_ascii_case("-inf")
        || trimmed.eq_ignore_ascii_case("+inf")
    {
        return Err(format!(
            "numeric value '{value}' is not supported by bounded NUMERIC({precision}, {scale})"
        ));
    }
    if precision == 0 || precision > 38 || scale < 0 || (scale as u8) > precision {
        return Err(format!("invalid NUMERIC({precision}, {scale}) bounds"));
    }

    let negative = trimmed.starts_with('-');
    let unsigned = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);
    let mut parts = unsigned.split('.');
    let integer_raw = parts.next().unwrap_or("");
    let fractional_raw = parts.next().unwrap_or("");
    if parts.next().is_some()
        || (integer_raw.is_empty() && fractional_raw.is_empty())
        || !integer_raw.bytes().all(|b| b.is_ascii_digit())
        || !fractional_raw.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!("invalid numeric literal '{value}'"));
    }

    let integer_part = if integer_raw.is_empty() { "0" } else { integer_raw };
    let scale_usize = scale as usize;
    let round_up = fractional_raw
        .as_bytes()
        .get(scale_usize)
        .is_some_and(|&digit| digit >= b'5');

    let mut normalized_fraction = String::with_capacity(scale_usize);
    if fractional_raw.len() >= scale_usize {
        normalized_fraction.push_str(&fractional_raw[..scale_usize]);
    } else {
        normalized_fraction.push_str(fractional_raw);
        while normalized_fraction.len() < scale_usize {
            normalized_fraction.push('0');
        }
    }

    let joined = format!("{integer_part}{normalized_fraction}");
    let mut magnitude = joined
        .parse::<i128>()
        .map_err(|_| format!("numeric field overflow: '{value}' exceeds NUMERIC({precision}, {scale})"))?;
    if round_up {
        magnitude = magnitude.checked_add(1).ok_or_else(|| {
            format!("numeric field overflow: '{value}' exceeds NUMERIC({precision}, {scale})")
        })?;
    }

    let limit = if precision == 38 {
        10_i128.pow(38)
    } else {
        10_i128.pow(u32::from(precision))
    };
    if magnitude >= limit {
        return Err(format!(
            "numeric field overflow: '{value}' exceeds NUMERIC({precision}, {scale})"
        ));
    }

    if negative && magnitude != 0 {
        Ok(-magnitude)
    } else {
        Ok(magnitude)
    }
}

/// Format an unscaled `i128` decimal value with `scale` fractional digits.
pub fn format_decimal_value(value: i128, scale: u8) -> String {
    if scale == 0 {
        return value.to_string();
    }

    let sign = if value < 0 { "-" } else { "" };
    let abs = value.unsigned_abs();
    let divisor = 10_u128.pow(u32::from(scale));
    let integer = abs / divisor;
    let fraction = abs % divisor;
    format!("{sign}{integer}.{fraction:0width$}", width = scale as usize)
}

pub(crate) fn extract_bool(scalar: &ScalarValue) -> Option<bool> {
    match scalar {
        ScalarValue::Boolean(v) => *v,
        _ => None,
    }
}

pub(crate) fn extract_i64(scalar: &ScalarValue) -> Option<i64> {
    match scalar {
        ScalarValue::Int64(v) => *v,
        ScalarValue::Int32(v) => v.map(i64::from),
        ScalarValue::Int16(v) => v.map(i64::from),
        ScalarValue::Int8(v) => v.map(i64::from),
        ScalarValue::UInt64(v) => v.and_then(|i| i64::try_from(i).ok()),
        ScalarValue::UInt32(v) => v.map(i64::from),
        ScalarValue::UInt16(v) => v.map(i64::from),
        ScalarValue::UInt8(v) => v.map(i64::from),
        ScalarValue::Date32(v) => v.map(i64::from),
        ScalarValue::Time64Microsecond(v) => *v,
        ScalarValue::TimestampMillisecond(v, _) => *v,
        ScalarValue::TimestampMicrosecond(v, _) => *v,
        ScalarValue::TimestampNanosecond(v, _) => *v,
        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => s.parse::<i64>().ok(),
        _ => None,
    }
}

pub(crate) fn extract_timestamp_micros(scalar: &ScalarValue) -> Option<i64> {
    match scalar {
        ScalarValue::TimestampSecond(v, _) => v.and_then(|value| value.checked_mul(1_000_000)),
        ScalarValue::TimestampMillisecond(v, _) => v.and_then(|value| value.checked_mul(1_000)),
        ScalarValue::TimestampMicrosecond(v, _) => *v,
        ScalarValue::TimestampNanosecond(v, _) => v.map(|value| value / 1_000),
        _ => extract_i64(scalar),
    }
}

pub(crate) fn extract_time_micros(scalar: &ScalarValue) -> Option<i64> {
    match scalar {
        ScalarValue::Time32Second(v) => v.and_then(|value| i64::from(value).checked_mul(1_000_000)),
        ScalarValue::Time32Millisecond(v) => {
            v.and_then(|value| i64::from(value).checked_mul(1_000))
        },
        ScalarValue::Time64Microsecond(v) => *v,
        ScalarValue::Time64Nanosecond(v) => v.map(|value| value / 1_000),
        _ => extract_i64(scalar),
    }
}

pub(crate) fn extract_f64(scalar: &ScalarValue) -> Option<f64> {
    match scalar {
        ScalarValue::Float64(v) => *v,
        ScalarValue::Float32(v) => v.map(f64::from),
        ScalarValue::Int64(v) => v.map(|i| i as f64),
        ScalarValue::Int32(v) => v.map(f64::from),
        ScalarValue::Int16(v) => v.map(f64::from),
        ScalarValue::Int8(v) => v.map(f64::from),
        ScalarValue::UInt64(v) => v.map(|i| i as f64),
        ScalarValue::UInt32(v) => v.map(f64::from),
        ScalarValue::UInt16(v) => v.map(f64::from),
        ScalarValue::UInt8(v) => v.map(f64::from),
        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => s.parse::<f64>().ok(),
        _ => None,
    }
}

pub(crate) fn extract_string(scalar: &ScalarValue) -> Option<String> {
    match scalar {
        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => Some(s.clone()),
        ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) => None,
        ScalarValue::Int64(v) => v.map(|i| i.to_string()),
        ScalarValue::UInt64(v) => v.map(|i| i.to_string()),
        ScalarValue::Int32(v) => v.map(|i| i.to_string()),
        ScalarValue::UInt32(v) => v.map(|i| i.to_string()),
        ScalarValue::Int16(v) => v.map(|i| i.to_string()),
        ScalarValue::UInt16(v) => v.map(|i| i.to_string()),
        ScalarValue::Int8(v) => v.map(|i| i.to_string()),
        ScalarValue::UInt8(v) => v.map(|i| i.to_string()),
        ScalarValue::Boolean(v) => v.map(|b| b.to_string()),
        ScalarValue::Float64(v) => v.map(|f| f.to_string()),
        ScalarValue::Float32(v) => v.map(|f| f.to_string()),
        ScalarValue::TimestampMillisecond(v, _) => v.map(|t| t.to_string()),
        ScalarValue::TimestampMicrosecond(v, _) => v.map(|t| t.to_string()),
        ScalarValue::TimestampNanosecond(v, _) => v.map(|t| t.to_string()),
        ScalarValue::Binary(Some(bytes)) | ScalarValue::LargeBinary(Some(bytes)) => {
            Some(String::from_utf8_lossy(bytes).to_string())
        },
        ScalarValue::Decimal128(Some(val), _precision, scale) => {
            Some(format_decimal_value(*val, (*scale).max(0) as u8))
        },
        _ => Some(format!("{scalar:?}")),
    }
}

#[cfg(test)]
mod tests {
    use datafusion_common::ScalarValue;
    use serde_json::json;

    use super::{json_value_to_scalar_for_column, scalar_to_json_for_column};
    use crate::models::datatypes::KalamDataType;

    #[test]
    fn test_uuid_roundtrip_as_text_scalar() {
        let value = json!("550e8400-e29b-41d4-a716-446655440000");
        let scalar =
            json_value_to_scalar_for_column(&value, &KalamDataType::Uuid).expect("uuid to scalar");

        match scalar {
            ScalarValue::Utf8(Some(ref v)) => {
                assert_eq!(v, "550e8400-e29b-41d4-a716-446655440000");
            },
            other => panic!("expected Utf8 for UUID, got {other:?}"),
        }

        let json_back =
            scalar_to_json_for_column(&scalar, &KalamDataType::Uuid).expect("scalar to uuid json");
        assert_eq!(json_back, value);
    }

    #[test]
    fn test_json_object_stored_as_utf8_and_roundtrips() {
        let value = json!({"route": ["a", "b"], "enabled": true});
        let scalar =
            json_value_to_scalar_for_column(&value, &KalamDataType::Json).expect("json to scalar");

        assert!(matches!(scalar, ScalarValue::Utf8(Some(_))));

        let json_back =
            scalar_to_json_for_column(&scalar, &KalamDataType::Json).expect("scalar to json");
        assert_eq!(json_back, value);
    }

    #[test]
    fn test_bytes_array_roundtrip() {
        let value = json!([1, 2, 255]);
        let scalar = json_value_to_scalar_for_column(&value, &KalamDataType::Bytes)
            .expect("bytes to scalar");

        assert!(
            matches!(scalar, ScalarValue::Binary(Some(ref bytes)) if bytes == &vec![1, 2, 255])
        );

        let json_back =
            scalar_to_json_for_column(&scalar, &KalamDataType::Bytes).expect("scalar to bytes");
        assert_eq!(json_back, value);
    }

    #[test]
    fn test_decimal_string_roundtrip() {
        let value = json!("200.75");
        let dtype = KalamDataType::Decimal {
            precision: 10,
            scale:     2,
        };

        let scalar = json_value_to_scalar_for_column(&value, &dtype).expect("decimal to scalar");
        assert!(matches!(scalar, ScalarValue::Decimal128(Some(20075), 10, 2)));

        let json_back = scalar_to_json_for_column(&scalar, &dtype).expect("scalar to decimal");
        assert_eq!(json_back, value);
    }

    #[test]
    fn test_numeric_string_to_int64_via_json_conversion() {
        let value = json!("42");
        let scalar =
            json_value_to_scalar_for_column(&value, &KalamDataType::BigInt).expect("bigint parse");
        assert!(matches!(scalar, ScalarValue::Int64(Some(0))));
    }

    #[test]
    fn test_temporal_json_numbers_become_microsecond_scalars() {
        let timestamp = json!(1_735_689_600_000_000_i64);
        let time = json!(45_296_123_456_i64);

        let scalar = json_value_to_scalar_for_column(&timestamp, &KalamDataType::Timestamp)
            .expect("timestamp scalar");
        assert!(matches!(
            scalar,
            ScalarValue::TimestampMicrosecond(Some(1_735_689_600_000_000), None)
        ));

        let scalar = json_value_to_scalar_for_column(&timestamp, &KalamDataType::DateTime)
            .expect("datetime scalar");
        assert!(matches!(
            scalar,
            ScalarValue::TimestampMicrosecond(Some(1_735_689_600_000_000), Some(_))
        ));

        let scalar =
            json_value_to_scalar_for_column(&time, &KalamDataType::Time).expect("time scalar");
        assert!(matches!(scalar, ScalarValue::Time64Microsecond(Some(45_296_123_456))));
    }

    #[test]
    fn test_timestamp_scalar_json_uses_microseconds() {
        let value = scalar_to_json_for_column(
            &ScalarValue::TimestampNanosecond(Some(1735689600000000000), None),
            &KalamDataType::Timestamp,
        )
        .expect("timestamp to json");

        assert_eq!(value, json!(1735689600000000_i64));
    }

    #[test]
    fn test_time_scalar_json_uses_microseconds() {
        let value = scalar_to_json_for_column(
            &ScalarValue::Time64Nanosecond(Some(45_296_123_456_789)),
            &KalamDataType::Time,
        )
        .expect("time to json");

        assert_eq!(value, json!(45_296_123_456_i64));
    }
}
