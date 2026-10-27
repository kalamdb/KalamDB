//! Function-call frames for every SDK.
//!
//! The bytes are the backend `kalamdb-serialization` domain codec. Hosts pass
//! JSON only as the FFI shape. They do not implement KDOM themselves.

use kalamdb_serialization::{
    decode_domain_frame_for_surface, domain_value_to_json, encode_domain_frame,
    json_to_domain_value, DomainDecodeBudget, DomainMessageKind, DomainValue, EndpointSurface,
};
use serde_json::{Number, Value as JsonValue};

const JS_MAX_SAFE_INTEGER: i64 = (1_i64 << 53) - 1;

/// One declared argument slot. `slot` is 1-based and must increase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineSlot {
    pub name: String,
    pub slot: u16,
    pub json: bool,
}

/// Encode a routine call as an `RpcRequest` frame.
pub fn encode_routine_request(
    arguments: &JsonValue,
    params: &[RoutineSlot],
) -> Result<Vec<u8>, String> {
    let payload = routine_payload(arguments, params)?;
    encode_domain_frame(DomainMessageKind::RpcRequest, 0, 0, &payload)
        .map_err(|err| err.to_string())
}

/// Decode an `RpcResult` frame into JSON.
pub fn decode_routine_result(bytes: &[u8]) -> Result<JsonValue, String> {
    let frame = decode_domain_frame_for_surface(
        bytes,
        EndpointSurface::PublicRpc,
        DomainDecodeBudget::default(),
    )
    .map_err(|err| err.to_string())?;
    if frame.kind != DomainMessageKind::RpcResult {
        return Err(format!("unexpected domain frame kind {:?}", frame.kind));
    }
    domain_value_to_json(&frame.payload).map_err(|err| err.to_string())
}

/// JSON arguments and a parameter list, both as JSON text.
pub fn encode_routine_request_json(
    arguments_json: &str,
    params_json: &str,
) -> Result<Vec<u8>, String> {
    let arguments = if arguments_json.is_empty() || arguments_json == "null" {
        JsonValue::Null
    } else {
        serde_json::from_str(arguments_json)
            .map_err(|err| format!("invalid arguments json: {err}"))?
    };
    let params = parse_slots(params_json)?;
    encode_routine_request(&arguments, &params)
}

/// Decode an `RpcResult` frame into JSON text.
pub fn decode_routine_result_json(bytes: &[u8]) -> Result<String, String> {
    Ok(decode_routine_result(bytes)?.to_string())
}

fn parse_slots(params_json: &str) -> Result<Vec<RoutineSlot>, String> {
    if params_json.is_empty() || params_json == "null" || params_json == "[]" {
        return Ok(Vec::new());
    }
    let value: JsonValue = serde_json::from_str(params_json)
        .map_err(|err| format!("invalid parameter json: {err}"))?;
    let Some(items) = value.as_array() else {
        return Err("routine parameters must be a JSON array".to_string());
    };
    let mut slots = Vec::with_capacity(items.len());
    for item in items {
        let name = item
            .get("name")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "routine parameter is missing name".to_string())?;
        let slot = item
            .get("slot")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| format!("routine parameter '{name}' is missing slot"))?;
        let slot = u16::try_from(slot).map_err(|_| format!("parameter slot {slot} exceeds u16"))?;
        let json = item.get("json").and_then(JsonValue::as_bool).unwrap_or(false);
        slots.push(RoutineSlot {
            name: name.to_string(),
            slot,
            json,
        });
    }
    Ok(slots)
}

fn routine_payload(arguments: &JsonValue, params: &[RoutineSlot]) -> Result<DomainValue, String> {
    if arguments.is_null() {
        return Ok(DomainValue::Void);
    }
    if params.is_empty() {
        return json_to_domain_value(arguments).map_err(|err| err.to_string());
    }
    let Some(object) = arguments.as_object() else {
        return Err("named routine arguments must be a JSON object".to_string());
    };
    let mut ordered = params.to_vec();
    ordered.sort_by_key(|param| param.slot);
    let mut fields = Vec::with_capacity(ordered.len());
    let mut previous = 0u16;
    for param in &ordered {
        if previous != 0 && param.slot <= previous {
            return Err(format!(
                "parameter slots must be strictly increasing ({previous} then {})",
                param.slot
            ));
        }
        previous = param.slot;
        let value = match object.get(&param.name) {
            None => DomainValue::Omitted,
            Some(JsonValue::Null) => DomainValue::SqlNull,
            Some(value) if param.json && (value.is_object() || value.is_array()) => {
                DomainValue::Json(value.to_string())
            },
            Some(value) => json_to_domain_value(value).map_err(|err| err.to_string())?,
        };
        fields.push((param.slot, value));
    }
    Ok(DomainValue::Composite(fields))
}

/// True when a decoded integer must be a JavaScript bigint rather than a number.
#[must_use]
pub fn json_number_needs_bigint(number: &Number) -> bool {
    number
        .as_i64()
        .is_some_and(|value| value.unsigned_abs() > JS_MAX_SAFE_INTEGER as u64)
        || number.as_u64().is_some_and(|value| value > JS_MAX_SAFE_INTEGER as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_slots_omit_missing_keys_and_round_trip_objects() {
        let arguments = serde_json::json!({"right": "b", "left": {"ok": true}});
        let params = vec![
            RoutineSlot {
                name: "left".into(),
                slot: 1,
                json: false,
            },
            RoutineSlot {
                name: "right".into(),
                slot: 2,
                json: false,
            },
        ];
        let frame = encode_routine_request(&arguments, &params).unwrap();
        assert_eq!(&frame[0..4], b"KDOM");
        assert_eq!(frame[8], DomainMessageKind::RpcRequest as u8);

        let mut result = frame;
        result[8] = DomainMessageKind::RpcResult as u8;
        let decoded = decode_routine_result(&result).unwrap();
        assert_eq!(decoded["1"]["ok"], true);
        assert_eq!(decoded["2"], "b");
    }

    #[test]
    fn wide_integer_survives_the_json_boundary() {
        let wide = 9_007_199_254_740_993_i64;
        let frame = encode_routine_request(&JsonValue::from(wide), &[]).unwrap();
        let mut result = frame;
        result[8] = DomainMessageKind::RpcResult as u8;
        let decoded = decode_routine_result(&result).unwrap();
        assert_eq!(decoded.as_i64(), Some(wide));
        assert!(json_number_needs_bigint(decoded.as_number().unwrap()));
    }
}
