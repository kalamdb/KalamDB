//! JSON host view of a [`DomainValue`].
//!
//! Function calls and JSON parameters share this mapping. WebSocket cells keep
//! their own object encoding in `ws_domain`.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{Map, Number, Value as JsonValue};

use crate::{DomainFileRef, DomainValue, SerializationError};

const BYTES_KEY: &str = "$kbytes";
const INT_KEY: &str = "$kint";

/// Convert JSON into a domain value.
///
/// A one-key object `{"$kbytes":"<base64>"}` is bytes. `{"$kint":"<decimal>"}`
/// is an integer wider than JSON numbers can carry through JavaScript.
pub fn json_to_domain_value(value: &JsonValue) -> Result<DomainValue, SerializationError> {
    Ok(match value {
        JsonValue::Null => DomainValue::SqlNull,
        JsonValue::Bool(flag) => DomainValue::Bool(*flag),
        JsonValue::Number(number) => number_to_domain(number)?,
        JsonValue::String(text) => DomainValue::Utf8(text.clone()),
        JsonValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(json_to_domain_value(item)?);
            }
            DomainValue::Array(out)
        },
        JsonValue::Object(fields) => {
            if let Some(marked) = marked_object(fields)? {
                marked
            } else {
                let mut out = Vec::with_capacity(fields.len());
                for (name, child) in fields {
                    out.push((name.clone(), json_to_domain_value(child)?));
                }
                DomainValue::Object(out)
            }
        },
    })
}

/// Convert a domain value into JSON.
pub fn domain_value_to_json(value: &DomainValue) -> Result<JsonValue, SerializationError> {
    Ok(match value {
        DomainValue::Omitted | DomainValue::SqlNull | DomainValue::JsonNull | DomainValue::Void => {
            JsonValue::Null
        },
        DomainValue::Bool(flag) => JsonValue::Bool(*flag),
        DomainValue::Int8(number) => JsonValue::from(*number),
        DomainValue::Int16(number) => JsonValue::from(*number),
        DomainValue::Int32(number) => JsonValue::from(*number),
        DomainValue::Int64(number) => JsonValue::from(*number),
        DomainValue::UInt8(number) => JsonValue::from(*number),
        DomainValue::UInt16(number) => JsonValue::from(*number),
        DomainValue::UInt32(number) => JsonValue::from(*number),
        DomainValue::UInt64(number) => JsonValue::from(*number),
        DomainValue::Float32(number) => json_float(f64::from(*number))?,
        DomainValue::Float64(number) => json_float(*number)?,
        DomainValue::Decimal128 {
            unscaled,
            precision,
            scale,
        } => serde_json::json!({
            "unscaled": unscaled.to_string(),
            "precision": precision,
            "scale": scale,
        }),
        DomainValue::Date32(number) => JsonValue::from(*number),
        DomainValue::TimeMicros(number)
        | DomainValue::TimestampMicros(number)
        | DomainValue::TimestampTzMicros(number) => JsonValue::from(*number),
        DomainValue::Uuid(bytes) => JsonValue::String(format_uuid(bytes)),
        DomainValue::Utf8(text) | DomainValue::Enum { label: text, .. } => {
            JsonValue::String(text.clone())
        },
        DomainValue::Bytes(bytes) => serde_json::json!({ BYTES_KEY: STANDARD.encode(bytes) }),
        DomainValue::Json(text) => {
            serde_json::from_str(text).unwrap_or_else(|_| JsonValue::String(text.clone()))
        },
        DomainValue::FileRef(file) => file_to_json(file),
        DomainValue::Object(fields) => {
            let mut map = Map::with_capacity(fields.len());
            for (name, field) in fields {
                map.insert(name.clone(), domain_value_to_json(field)?);
            }
            JsonValue::Object(map)
        },
        DomainValue::Composite(fields) => {
            let mut map = Map::with_capacity(fields.len());
            for (slot, field) in fields {
                map.insert(slot.to_string(), domain_value_to_json(field)?);
            }
            JsonValue::Object(map)
        },
        DomainValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(domain_value_to_json(item)?);
            }
            JsonValue::Array(out)
        },
        DomainValue::RowSet(rows) => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                out.push(domain_value_to_json(&DomainValue::Composite(row.clone()))?);
            }
            JsonValue::Array(out)
        },
    })
}

fn marked_object(
    fields: &Map<String, JsonValue>,
) -> Result<Option<DomainValue>, SerializationError> {
    if fields.len() != 1 {
        return Ok(None);
    }
    if let Some(JsonValue::String(encoded)) = fields.get(BYTES_KEY) {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|err| SerializationError::Decode(format!("invalid byte payload: {err}")))?;
        return Ok(Some(DomainValue::Bytes(bytes)));
    }
    if let Some(JsonValue::String(text)) = fields.get(INT_KEY) {
        if let Ok(value) = text.parse::<i64>() {
            return Ok(Some(integer_domain(value)));
        }
        if let Ok(value) = text.parse::<u64>() {
            return Ok(Some(DomainValue::UInt64(value)));
        }
        return Err(SerializationError::Decode(format!("invalid integer payload {text}")));
    }
    Ok(None)
}

fn number_to_domain(number: &Number) -> Result<DomainValue, SerializationError> {
    if let Some(value) = number.as_i64() {
        return Ok(integer_domain(value));
    }
    if let Some(value) = number.as_u64() {
        return Ok(DomainValue::UInt64(value));
    }
    let Some(value) = number.as_f64() else {
        return Err(SerializationError::Encode(format!("unsupported JSON number {number}")));
    };
    Ok(DomainValue::Float64(value))
}

fn integer_domain(value: i64) -> DomainValue {
    if i64::from(i32::MIN) <= value && value <= i64::from(i32::MAX) {
        DomainValue::Int32(value as i32)
    } else {
        DomainValue::Int64(value)
    }
}

fn json_float(value: f64) -> Result<JsonValue, SerializationError> {
    Number::from_f64(value)
        .map(JsonValue::Number)
        .ok_or_else(|| SerializationError::Encode("non-finite floating point value".into()))
}

fn file_to_json(file: &DomainFileRef) -> JsonValue {
    serde_json::json!({
        "id": file.id,
        "sub": file.sub,
        "name": file.name,
        "size": file.size,
        "mime": file.mime,
        "sha256": file.sha256,
        "shard": file.shard,
    })
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip_through_the_marker() {
        let value = DomainValue::Bytes(vec![1, 2, 255]);
        let json = domain_value_to_json(&value).unwrap();
        assert_eq!(json_to_domain_value(&json).unwrap(), value);
    }

    #[test]
    fn wide_integer_marker_is_int64() {
        let json = serde_json::json!({"$kint": "9007199254740993"});
        assert_eq!(json_to_domain_value(&json).unwrap(), DomainValue::Int64(9_007_199_254_740_993));
    }
}
