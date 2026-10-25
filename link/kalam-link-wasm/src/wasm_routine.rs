//! JavaScript boundary for the shared routine frame codec.

use link_common::decode_routine_result as decode_routine_frame;
use link_common::{encode_routine_request_json, json_number_needs_bigint};
use serde_json::Value as JsonValue;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = encodeRoutineRequest)]
pub fn encode_routine_request(arguments_json: &str, params_json: &str) -> Result<Vec<u8>, JsValue> {
    encode_routine_request_json(arguments_json, params_json).map_err(js_error)
}

#[wasm_bindgen(js_name = decodeRoutineResult)]
pub fn decode_routine_result(frame: &[u8]) -> Result<JsValue, JsValue> {
    let value = decode_routine_frame(frame).map_err(js_error)?;
    json_to_js(&value)
}

fn json_to_js(value: &JsonValue) -> Result<JsValue, JsValue> {
    let js = match value {
        JsonValue::Null => JsValue::NULL,
        JsonValue::Bool(flag) => JsValue::from(*flag),
        JsonValue::Number(number) if json_number_needs_bigint(number) => {
            let text = number.to_string();
            js_sys::BigInt::new(&JsValue::from_str(&text))
                .map_err(|_| js_error(format!("invalid bigint {text}")))?
                .into()
        },
        JsonValue::Number(number) => {
            JsValue::from(number.as_f64().ok_or_else(|| js_error("non-finite number".into()))?)
        },
        JsonValue::String(text) => JsValue::from_str(text),
        JsonValue::Array(items) => {
            let array = js_sys::Array::new_with_length(items.len() as u32);
            for (index, item) in items.iter().enumerate() {
                array.set(index as u32, json_to_js(item)?);
            }
            array.into()
        },
        JsonValue::Object(fields) => {
            if let Some(bytes) = bytes_marker(fields) {
                return Ok(js_sys::Uint8Array::from(bytes.as_slice()).into());
            }
            if let Some(text) = int_marker(fields) {
                return js_sys::BigInt::new(&JsValue::from_str(text))
                    .map(Into::into)
                    .map_err(|_| js_error(format!("invalid bigint {text}")));
            }
            let object = js_sys::Object::new();
            for (key, item) in fields {
                js_sys::Reflect::set(&object, &JsValue::from_str(key), &json_to_js(item)?)
                    .map_err(|_| js_error(format!("failed to set field {key}")))?;
            }
            object.into()
        },
    };
    Ok(js)
}

fn bytes_marker(fields: &serde_json::Map<String, JsonValue>) -> Option<Vec<u8>> {
    if fields.len() != 1 {
        return None;
    }
    let text = fields.get("$kbytes")?.as_str()?;
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text).ok()
}

fn int_marker(fields: &serde_json::Map<String, JsonValue>) -> Option<&str> {
    if fields.len() != 1 {
        return None;
    }
    fields.get("$kint")?.as_str()
}

fn js_error(message: String) -> JsValue {
    JsValue::from_str(&message)
}
