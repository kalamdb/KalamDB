//! In-memory function JS-boundary codec.
//!
//! Nested in-process calls pass these bytes; this is not the durable catalog
//! [`crate::encode_object`] path.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{Result, SerializationError},
    object::{
        decode_envelope, decode_flexbuffers, encode_envelope, encode_flexbuffers, ObjectKind,
    },
};

const FUNCTION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct FunctionTransfer {
    contract_hash: String,
    value:         Value,
}

/// Encode a function value tagged with the contract hash that produced it.
pub fn encode_function_value(contract_hash: &str, value: &Value) -> Result<Vec<u8>> {
    let payload = encode_flexbuffers(&FunctionTransfer {
        contract_hash: contract_hash.to_string(),
        value:         value.clone(),
    })?;
    Ok(encode_envelope(ObjectKind::Function, FUNCTION_SCHEMA_VERSION, &payload)?.into_bytes())
}

/// Decode a function value. `expected_hash` rejects a stale contract.
pub fn decode_function_value(bytes: &[u8], expected_hash: &str) -> Result<Value> {
    let (_header, payload) = decode_envelope(bytes, ObjectKind::Function)?;
    let transfer: FunctionTransfer = decode_flexbuffers(payload)?;
    if transfer.contract_hash != expected_hash {
        return Err(SerializationError::Decode(format!(
            "function contract hash mismatch: expected {expected_hash}, got {}",
            transfer.contract_hash
        )));
    }
    Ok(transfer.value)
}

/// Reuses the last encoded buffer when hash and JSON are unchanged.
#[derive(Debug, Default)]
pub struct FunctionValueEncoder {
    last_hash:  String,
    last_value: Option<Value>,
    last_bytes: Option<Arc<[u8]>>,
}

impl FunctionValueEncoder {
    pub fn encode(&mut self, contract_hash: &str, value: &Value) -> Result<Arc<[u8]>> {
        if self.last_hash == contract_hash && self.last_value.as_ref() == Some(value) {
            if let Some(bytes) = &self.last_bytes {
                return Ok(Arc::clone(bytes));
            }
        }
        let encoded = encode_function_value(contract_hash, value)?;
        let encoded: Arc<[u8]> = encoded.into();
        self.last_hash = contract_hash.to_string();
        self.last_value = Some(value.clone());
        self.last_bytes = Some(Arc::clone(&encoded));
        Ok(encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_reuses_buffer_for_same_hash_and_value() {
        let mut encoder = FunctionValueEncoder::default();
        let value = json!({"id": 7});
        let first = encoder.encode("abc", &value).unwrap();
        let second = encoder.encode("abc", &value).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(decode_function_value(&first, "abc").unwrap(), value);
    }

    #[test]
    fn contract_hash_mismatch_forces_reencode() {
        let mut encoder = FunctionValueEncoder::default();
        let value = json!(1);
        let first = encoder.encode("v1", &value).unwrap();
        let second = encoder.encode("v2", &value).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        let err = decode_function_value(&second, "v1").unwrap_err();
        assert!(err.to_string().contains("contract hash mismatch"), "{err}");
    }
}
