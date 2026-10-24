//! Canonical cross-language scalar, presence, and structured field-error mappings (`T005`).
//!
//! Defines the single semantic truth for wide integers (`BIGINT`), exact decimals
//! (`NUMERIC(p, s)`), precision-preserving temporals (`DATE`, `TIME`, `TIMESTAMP`,
//! `TIMESTAMPTZ`), binary payloads (`BYTEA`/`BYTES`), `FILE` references, and the
//! 4-state presence model (`Omitted`, `SqlNull`, `JsonNull`, `Value(T)`).

use kalamdb_commons::KalamDataType;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Four-state field presence across RPC, table insert/patch, and live changes (`FR-040`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValuePresence<T> {
    /// Field was omitted from an input/patch object; default or partial-patch semantics apply.
    Omitted,
    /// SQL `NULL` value; distinct from `Omitted` and from `JsonNull`.
    SqlNull,
    /// JSON `null` literal inside a `JSON`/`JSONB` column or parameter.
    JsonNull,
    /// Present non-null domain value.
    Value(T),
}

impl<T> ValuePresence<T> {
    #[inline]
    pub fn is_omitted(&self) -> bool {
        matches!(self, Self::Omitted)
    }

    #[inline]
    pub fn is_sql_null(&self) -> bool {
        matches!(self, Self::SqlNull)
    }

    #[inline]
    pub fn is_json_null(&self) -> bool {
        matches!(self, Self::JsonNull)
    }
}

/// Checked cross-language scalar representation (`FR-005`, `FR-006`, `FR-040`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CheckedContractScalar {
    Boolean(bool),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    /// Exact fixed-point decimal with validated precision and scale (`i128` unscaled value).
    Decimal {
        unscaled:  i128,
        precision: u8,
        scale:     i8,
    },
    Text(String),
    Uuid([u8; 16]),
    Bytes(Vec<u8>),
    /// Days since 1970-01-01 (`DATE`).
    Date32(i32),
    /// Microseconds since midnight (`TIME`).
    TimeMicros(i64),
    /// Microseconds since Unix epoch without timezone (`TIMESTAMP`).
    TimestampMicros(i64),
    /// Microseconds since Unix epoch with UTC instant semantics (`TIMESTAMPTZ`).
    TimestampTzMicros(i64),
    /// Checked JSON value (where `JsonValue::Null` is represented via `ValuePresence::JsonNull`).
    Json(JsonValue),
}

/// Structured validation / boundary error with exact nested field path (`FR-006`, `FR-069`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractFieldError {
    pub code:       String,
    pub field_path: String,
    pub message:    String,
    pub retryable:  bool,
}

impl ContractFieldError {
    pub fn invalid(field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code:       "INVALID_FIELD_VALUE".to_string(),
            field_path: field_path.into(),
            message:    message.into(),
            retryable:  false,
        }
    }

    pub fn out_of_range(field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code:       "NUMERIC_VALUE_OUT_OF_RANGE".to_string(),
            field_path: field_path.into(),
            message:    message.into(),
            retryable:  false,
        }
    }

    pub fn null_violation(field_path: impl Into<String>) -> Self {
        let path = field_path.into();
        Self {
            code:       "NOT_NULL_VIOLATION".to_string(),
            message:    format!("field '{path}' cannot be SQL NULL"),
            field_path: path,
            retryable:  false,
        }
    }
}

/// Validate and parse a JSON-encoded value into a [`ValuePresence<CheckedContractScalar>`]
/// without lossy float conversions for `BIGINT` or `NUMERIC`.
pub fn check_json_scalar(
    raw: Option<&JsonValue>,
    data_type: &KalamDataType,
    not_null: bool,
    allow_omitted: bool,
    field_path: &str,
) -> Result<ValuePresence<CheckedContractScalar>, ContractFieldError> {
    let Some(value) = raw else {
        if allow_omitted {
            return Ok(ValuePresence::Omitted);
        }
        if not_null {
            return Err(ContractFieldError {
                code:       "MISSING_REQUIRED_FIELD".to_string(),
                field_path: field_path.to_string(),
                message:    format!("required field '{field_path}' is missing"),
                retryable:  false,
            });
        }
        return Ok(ValuePresence::SqlNull);
    };

    if value.is_null() {
        if not_null {
            return Err(ContractFieldError::null_violation(field_path));
        }
        return Ok(ValuePresence::SqlNull);
    }

    let checked = match data_type {
        KalamDataType::Boolean => match value {
            JsonValue::Bool(b) => CheckedContractScalar::Boolean(*b),
            _ => return Err(ContractFieldError::invalid(field_path, "expected boolean value")),
        },
        KalamDataType::SmallInt => CheckedContractScalar::Int16(parse_checked_i64(
            value,
            i64::from(i16::MIN),
            i64::from(i16::MAX),
            true,
            field_path,
        )? as i16),
        KalamDataType::Int => CheckedContractScalar::Int32(parse_checked_i64(
            value,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
            true,
            field_path,
        )? as i32),
        KalamDataType::BigInt => CheckedContractScalar::Int64(parse_checked_i64(
            value,
            i64::MIN,
            i64::MAX,
            false,
            field_path,
        )?),
        KalamDataType::Float => {
            let n = value.as_f64().ok_or_else(|| {
                ContractFieldError::invalid(field_path, "expected finite 32-bit float")
            })?;
            if !n.is_finite() || n < f64::from(f32::MIN) || n > f64::from(f32::MAX) {
                return Err(ContractFieldError::out_of_range(
                    field_path,
                    "float32 value out of range",
                ));
            }
            CheckedContractScalar::Float32(n as f32)
        },
        KalamDataType::Double => {
            let n = value.as_f64().ok_or_else(|| {
                ContractFieldError::invalid(field_path, "expected finite 64-bit float")
            })?;
            if !n.is_finite() {
                return Err(ContractFieldError::out_of_range(
                    field_path,
                    "float64 value must be finite",
                ));
            }
            CheckedContractScalar::Float64(n)
        },
        KalamDataType::Decimal { precision, scale } => {
            let text = match value {
                JsonValue::String(s) => s.as_str(),
                _ => {
                    return Err(ContractFieldError::invalid(
                        field_path,
                        "DECIMAL/NUMERIC requires an exact string representation",
                    ));
                },
            };
            let unscaled = parse_checked_decimal128(text, *precision, *scale as i8, field_path)?;
            CheckedContractScalar::Decimal {
                unscaled,
                precision: *precision,
                scale: *scale as i8,
            }
        },
        KalamDataType::Text => match value {
            JsonValue::String(s) => CheckedContractScalar::Text(s.clone()),
            _ => return Err(ContractFieldError::invalid(field_path, "expected string")),
        },
        KalamDataType::Uuid => {
            let s = value
                .as_str()
                .ok_or_else(|| ContractFieldError::invalid(field_path, "expected UUID string"))?;
            let parsed = parse_uuid_bytes(s, field_path)?;
            CheckedContractScalar::Uuid(parsed)
        },
        KalamDataType::Date => {
            let s = value.as_str().ok_or_else(|| {
                ContractFieldError::invalid(field_path, "expected YYYY-MM-DD date string")
            })?;
            let days = parse_date32_days(s, field_path)?;
            CheckedContractScalar::Date32(days)
        },
        KalamDataType::Time => {
            let s = value.as_str().ok_or_else(|| {
                ContractFieldError::invalid(field_path, "expected HH:MM:SS[.ffffff] time string")
            })?;
            let micros = parse_time_micros(s, field_path)?;
            CheckedContractScalar::TimeMicros(micros)
        },
        KalamDataType::Timestamp | KalamDataType::DateTime => {
            let s = value.as_str().ok_or_else(|| {
                ContractFieldError::invalid(field_path, "expected ISO-8601 timestamp string")
            })?;
            let micros = parse_checked_timestamp_micros(s, field_path)?;
            CheckedContractScalar::TimestampTzMicros(micros)
        },
        KalamDataType::Json => {
            if let JsonValue::Object(map) = value {
                if map.len() == 1 {
                    if let Some(inner) = map.get("json") {
                        if inner.is_null() {
                            return Ok(ValuePresence::JsonNull);
                        }
                        return Ok(ValuePresence::Value(CheckedContractScalar::Json(
                            inner.clone(),
                        )));
                    }
                }
            }
            CheckedContractScalar::Json(value.clone())
        },
        KalamDataType::Bytes => {
            let arr = value
                .as_array()
                .ok_or_else(|| ContractFieldError::invalid(field_path, "expected byte array"))?;
            let mut bytes = Vec::with_capacity(arr.len().min(1024));
            for (idx, item) in arr.iter().enumerate() {
                let b = item.as_u64().filter(|v| *v <= 255).ok_or_else(|| {
                    ContractFieldError::invalid(
                        format!("{field_path}[{idx}]"),
                        "expected byte value 0..=255",
                    )
                })?;
                bytes.push(b as u8);
            }
            CheckedContractScalar::Bytes(bytes)
        },
        other => {
            return Err(ContractFieldError::invalid(
                field_path,
                format!("unsupported scalar mapping for {other:?}"),
            ));
        },
    };

    Ok(ValuePresence::Value(checked))
}

fn parse_checked_i64(
    value: &JsonValue,
    min: i64,
    max: i64,
    allow_unsafe_js_number: bool,
    field_path: &str,
) -> Result<i64, ContractFieldError> {
    const MAX_SAFE_JS_INT: i64 = (1i64 << 53) - 1;
    let parsed = match value {
        JsonValue::Number(n) => {
            let Some(i) = n.as_i64() else {
                return Err(ContractFieldError::invalid(
                    field_path,
                    "expected exact integer value",
                ));
            };
            if !allow_unsafe_js_number && i.unsigned_abs() > (MAX_SAFE_JS_INT as u64) {
                return Err(ContractFieldError::out_of_range(
                    field_path,
                    "64-bit integer exceeds IEEE-754 safe integer range; pass as string or bigint",
                ));
            }
            i
        },
        JsonValue::String(s) => s.parse::<i64>().map_err(|_| {
            ContractFieldError::out_of_range(
                field_path,
                format!("invalid or out-of-range integer '{s}'"),
            )
        })?,
        _ => {
            return Err(ContractFieldError::invalid(
                field_path,
                "expected integer or canonical integer string",
            ));
        },
    };
    if parsed < min || parsed > max {
        return Err(ContractFieldError::out_of_range(
            field_path,
            format!("integer {parsed} outside range [{min}, {max}]"),
        ));
    }
    Ok(parsed)
}

pub fn parse_checked_decimal128(
    text: &str,
    precision: u8,
    scale: i8,
    field_path: &str,
) -> Result<i128, ContractFieldError> {
    if text.is_empty() || precision == 0 || precision > 38 || scale < 0 || (scale as u8) > precision
    {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("invalid decimal specification ({precision}, {scale}) or empty text"),
        ));
    }
    let negative = text.starts_with('-');
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body.is_empty() {
        return Err(ContractFieldError::invalid(field_path, "empty decimal digits"));
    }
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => {
            if f.contains('.') {
                return Err(ContractFieldError::invalid(field_path, "multiple decimal points"));
            }
            (i, f)
        },
        None => (body, ""),
    };
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("invalid decimal literal '{text}'"),
        ));
    }
    let scale_usize = scale as usize;
    if frac_part.len() > scale_usize {
        return Err(ContractFieldError::out_of_range(
            field_path,
            format!(
                "decimal '{text}' has {} fractional digits, exceeding scale {scale}",
                frac_part.len()
            ),
        ));
    }
    let trimmed_int = int_part.trim_start_matches('0');
    let max_int_digits = (precision as usize).saturating_sub(scale_usize);
    if trimmed_int.len() > max_int_digits {
        return Err(ContractFieldError::out_of_range(
            field_path,
            format!("decimal '{text}' exceeds precision {precision} with scale {scale}"),
        ));
    }
    let mut combined = String::with_capacity(trimmed_int.len() + scale_usize + 1);
    if trimmed_int.is_empty() {
        combined.push('0');
    } else {
        combined.push_str(trimmed_int);
    }
    combined.push_str(frac_part);
    for _ in frac_part.len()..scale_usize {
        combined.push('0');
    }
    let unsigned = combined.parse::<i128>().map_err(|_| {
        ContractFieldError::out_of_range(field_path, format!("decimal '{text}' overflows i128"))
    })?;
    Ok(if negative { -unsigned } else { unsigned })
}

fn parse_uuid_bytes(text: &str, field_path: &str) -> Result<[u8; 16], ContractFieldError> {
    if text.len() != 36 {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("invalid UUID length in '{text}'"),
        ));
    }
    let bytes = text.as_bytes();
    for &dash in &[8usize, 13, 18, 23] {
        if bytes[dash] != b'-' {
            return Err(ContractFieldError::invalid(
                field_path,
                format!("invalid UUID format in '{text}'"),
            ));
        }
    }
    let mut out = [0u8; 16];
    let mut out_idx = 0usize;
    let mut i = 0usize;
    while i < 36 {
        if bytes[i] == b'-' {
            i += 1;
            continue;
        }
        let hi = (bytes[i] as char).to_digit(16).ok_or_else(|| {
            ContractFieldError::invalid(field_path, format!("invalid UUID hex in '{text}'"))
        })?;
        let lo = (bytes[i + 1] as char).to_digit(16).ok_or_else(|| {
            ContractFieldError::invalid(field_path, format!("invalid UUID hex in '{text}'"))
        })?;
        out[out_idx] = ((hi << 4) | lo) as u8;
        out_idx += 1;
        i += 2;
    }
    Ok(out)
}

fn parse_date32_days(text: &str, field_path: &str) -> Result<i32, ContractFieldError> {
    let parts: Vec<&str> = text.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("invalid date '{text}'; expected YYYY-MM-DD"),
        ));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| ContractFieldError::invalid(field_path, "invalid year"))?;
    let month: u32 = parts[1]
        .parse()
        .map_err(|_| ContractFieldError::invalid(field_path, "invalid month"))?;
    let day: u32 = parts[2]
        .parse()
        .map_err(|_| ContractFieldError::invalid(field_path, "invalid day"))?;
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
            if leap {
                29
            } else {
                28
            }
        },
        _ => {
            return Err(ContractFieldError::invalid(
                field_path,
                format!("month {month} out of range 1..=12"),
            ));
        },
    };
    if day == 0 || day > max_day {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("day {day} out of range 1..={max_day}"),
        ));
    }
    // Howard Hinnant civil-from-days algorithm relative to 1970-01-01
    let y = year - i32::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = month;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(era * 146097 + (doe as i32) - 719468)
}

fn parse_time_micros(text: &str, field_path: &str) -> Result<i64, ContractFieldError> {
    let (hms, frac) = text.split_once('.').unwrap_or((text, ""));
    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() != 3 || parts[0].len() != 2 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("invalid time '{text}'; expected HH:MM:SS[.ffffff]"),
        ));
    }
    let hour: u32 = parts[0]
        .parse()
        .map_err(|_| ContractFieldError::invalid(field_path, "invalid hour"))?;
    let min: u32 = parts[1]
        .parse()
        .map_err(|_| ContractFieldError::invalid(field_path, "invalid minute"))?;
    let sec: u32 = parts[2]
        .parse()
        .map_err(|_| ContractFieldError::invalid(field_path, "invalid second"))?;
    if hour > 23 || min > 59 || sec > 59 {
        return Err(ContractFieldError::invalid(field_path, format!("time '{text}' out of range")));
    }
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("invalid fractional seconds in '{text}'"),
        ));
    }
    if frac.len() > 6 {
        let extra = &frac[6..];
        if extra.chars().any(|c| c != '0') {
            return Err(ContractFieldError::out_of_range(
                field_path,
                "timestamp/time sub-microsecond nanoseconds cannot be represented losslessly",
            ));
        }
    }
    let mut micros_str = String::from(&frac[..frac.len().min(6)]);
    while micros_str.len() < 6 {
        micros_str.push('0');
    }
    let frac_micros: i64 = if micros_str.is_empty() {
        0
    } else {
        micros_str.parse().unwrap_or(0)
    };
    let total_secs = i64::from(hour) * 3_600 + i64::from(min) * 60 + i64::from(sec);
    Ok(total_secs * 1_000_000 + frac_micros)
}

fn parse_checked_timestamp_micros(text: &str, field_path: &str) -> Result<i64, ContractFieldError> {
    let normalized = text.strip_suffix('Z').or_else(|| text.strip_suffix('z')).unwrap_or(text);
    let (date_part, time_part) = normalized
        .split_once('T')
        .or_else(|| normalized.split_once(' '))
        .ok_or_else(|| {
            ContractFieldError::invalid(
                field_path,
                format!("invalid timestamp '{text}'; expected ISO-8601 date and time"),
            )
        })?;
    let (time_only, offset_micros) = if let Some((t, off)) = time_part.split_once('+') {
        (t, -parse_tz_offset_micros(off, field_path)?)
    } else if let Some((t, off)) = time_part.split_once('-') {
        (t, parse_tz_offset_micros(off, field_path)?)
    } else {
        (time_part, 0i64)
    };
    let days = i64::from(parse_date32_days(date_part, field_path)?);
    let day_micros = parse_time_micros(time_only, field_path)?;
    Ok(days * 86_400_000_000 + day_micros + offset_micros)
}

fn parse_tz_offset_micros(offset: &str, field_path: &str) -> Result<i64, ContractFieldError> {
    let (hh, mm) = offset.split_once(':').unwrap_or((offset, "00"));
    let hours: i64 = hh.parse().map_err(|_| {
        ContractFieldError::invalid(field_path, format!("invalid timezone offset '{offset}'"))
    })?;
    let mins: i64 = mm.parse().map_err(|_| {
        ContractFieldError::invalid(field_path, format!("invalid timezone offset '{offset}'"))
    })?;
    if hours > 14 || mins > 59 {
        return Err(ContractFieldError::invalid(
            field_path,
            format!("timezone offset '{offset}' out of range"),
        ));
    }
    Ok((hours * 3_600 + mins * 60) * 1_000_000)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn distinguishes_omitted_sql_null_and_json_null() {
        let omitted =
            check_json_scalar(None, &KalamDataType::Json, false, true, "payload").unwrap();
        assert_eq!(omitted, ValuePresence::Omitted);

        let sql_null =
            check_json_scalar(Some(&JsonValue::Null), &KalamDataType::Json, false, true, "payload")
                .unwrap();
        assert_eq!(sql_null, ValuePresence::SqlNull);

        let wrapped_json_null = check_json_scalar(
            Some(&json!({"json": null})),
            &KalamDataType::Json,
            true,
            false,
            "payload",
        )
        .unwrap();
        assert_eq!(wrapped_json_null, ValuePresence::JsonNull);
    }

    #[test]
    fn checks_wide_int64_and_exact_decimal_bounds() {
        let valid_i64 = check_json_scalar(
            Some(&json!("-9223372036854775808")),
            &KalamDataType::BigInt,
            true,
            false,
            "id",
        )
        .unwrap();
        assert_eq!(valid_i64, ValuePresence::Value(CheckedContractScalar::Int64(i64::MIN)));

        let overflow_i64 = check_json_scalar(
            Some(&json!("9223372036854775808")),
            &KalamDataType::BigInt,
            true,
            false,
            "id",
        )
        .unwrap_err();
        assert_eq!(overflow_i64.code, "NUMERIC_VALUE_OUT_OF_RANGE");

        let dec_ty = KalamDataType::Decimal {
            precision: 10,
            scale:     2,
        };
        let valid_dec =
            check_json_scalar(Some(&json!("-12345678.95")), &dec_ty, true, false, "price").unwrap();
        assert_eq!(
            valid_dec,
            ValuePresence::Value(CheckedContractScalar::Decimal {
                unscaled:  -1_234_567_895,
                precision: 10,
                scale:     2,
            })
        );

        let excess_scale =
            check_json_scalar(Some(&json!("1.234")), &dec_ty, true, false, "price").unwrap_err();
        assert_eq!(excess_scale.code, "NUMERIC_VALUE_OUT_OF_RANGE");
    }

    #[test]
    fn preserves_microsecond_timestamps_and_rejects_nanosecond_truncation() {
        let valid = check_json_scalar(
            Some(&json!("2026-10-23T22:13:45.123456Z")),
            &KalamDataType::Timestamp,
            true,
            false,
            "created_at",
        )
        .unwrap();
        assert!(matches!(
            valid,
            ValuePresence::Value(CheckedContractScalar::TimestampTzMicros(_))
        ));

        let lossy_nanos = check_json_scalar(
            Some(&json!("2026-10-23T22:13:45.123456789Z")),
            &KalamDataType::Timestamp,
            true,
            false,
            "created_at",
        )
        .unwrap_err();
        assert_eq!(lossy_nanos.code, "NUMERIC_VALUE_OUT_OF_RANGE");
    }
}
