//! Generic object encode/decode using the central envelope.

use serde::{de::DeserializeOwned, Serialize};

use super::{
    begin_envelope, decode_envelope, decode_flexbuffers, encode_envelope, encode_flexbuffers,
    finish_envelope, EncodedObject, ObjectKind,
};
use crate::error::{Result, SerializationError};

/// Default schema version for generic objects without a caller-supplied version.
pub const GENERIC_SCHEMA_VERSION: u16 = 1;
const STRING_LIST_BINARY_SCHEMA_VERSION: u16 = 2;

/// Encode a serde value as a generic persisted object.
pub fn encode_object<T: Serialize + ?Sized>(value: &T) -> Result<EncodedObject> {
    encode_object_versioned(value, GENERIC_SCHEMA_VERSION)
}

/// Encode a serde value with an explicit logical schema version.
pub fn encode_object_versioned<T: Serialize + ?Sized>(
    value: &T,
    schema_version: u16,
) -> Result<EncodedObject> {
    let payload = encode_flexbuffers(value)?;
    encode_envelope(ObjectKind::Generic, schema_version, &payload)
}

/// Decode a generic persisted object in the current envelope.
pub fn decode_object<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let (_header, payload) = decode_envelope(bytes, ObjectKind::Generic)?;
    decode_flexbuffers(payload)
}

/// Encode a list of strings (non-unique secondary-index PK arrays).
pub fn encode_string_list(values: &[String]) -> Result<EncodedObject> {
    let count = u32::try_from(values.len())
        .map_err(|_| SerializationError::Encode("string list exceeds u32 count".to_string()))?;
    let payload_bytes: usize = 4 + values.iter().map(|s| 4 + s.len()).sum::<usize>();
    let mut buf = begin_envelope(
        ObjectKind::Generic,
        STRING_LIST_BINARY_SCHEMA_VERSION,
        0,
        payload_bytes,
    );
    buf.extend_from_slice(&count.to_le_bytes());
    for s in values {
        let len = u32::try_from(s.len()).map_err(|_| {
            SerializationError::Encode("string list entry exceeds u32 length".to_string())
        })?;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }
    finish_envelope(buf)
}

/// Decode a string list from the object envelope.
pub fn decode_string_list(bytes: &[u8]) -> Result<Vec<String>> {
    let (header, payload) = decode_envelope(bytes, ObjectKind::Generic)?;
    if header.schema_version != STRING_LIST_BINARY_SCHEMA_VERSION {
        return Err(SerializationError::Decode(format!(
            "unsupported string list schema version {}",
            header.schema_version
        )));
    }
    if payload.len() < 4 {
        return Err(SerializationError::Decode(
            "truncated binary string list count".to_string(),
        ));
    }
    let count = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let mut cursor = 4usize;
    let mut out = Vec::with_capacity(count.min((payload.len() - 4) / 4));
    for _ in 0..count {
        let end_len = cursor.checked_add(4).ok_or_else(|| {
            SerializationError::Decode("string list length overflow".to_string())
        })?;
        let len_slice = payload.get(cursor..end_len).ok_or_else(|| {
            SerializationError::Decode("truncated binary string list entry length".to_string())
        })?;
        let len = u32::from_le_bytes([len_slice[0], len_slice[1], len_slice[2], len_slice[3]])
            as usize;
        cursor = end_len;
        let end_str = cursor.checked_add(len).ok_or_else(|| {
            SerializationError::Decode("string list entry overflow".to_string())
        })?;
        let str_bytes = payload.get(cursor..end_str).ok_or_else(|| {
            SerializationError::Decode("truncated binary string list entry".to_string())
        })?;
        let s = std::str::from_utf8(str_bytes).map_err(|e| {
            SerializationError::Decode(format!("invalid utf-8 in string list: {e}"))
        })?;
        out.push(s.to_owned());
        cursor = end_str;
    }
    if cursor != payload.len() {
        return Err(SerializationError::Decode(
            "trailing bytes in binary string list".to_string(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::object::encode_flexbuffers;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Sample {
        id:   String,
        n:    i64,
        flag: bool,
    }

    fn sample() -> Sample {
        Sample {
            id:   "ns.types".to_string(),
            n:    42,
            flag: true,
        }
    }

    #[test]
    fn generic_object_roundtrip() {
        let value = sample();
        let encoded = encode_object(&value).unwrap();
        let decoded: Sample = decode_object(encoded.as_slice()).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn decode_object_rejects_unenveloped_flexbuffers() {
        let value = sample();
        let legacy = encode_flexbuffers(&value).unwrap();
        assert!(decode_object::<Sample>(&legacy).is_err());
        assert!(decode_string_list(&legacy).is_err());
    }

    #[test]
    fn string_list_roundtrip_rejects_json() {
        let values = vec!["a".to_string(), "b".to_string()];
        let encoded = encode_string_list(&values).unwrap();
        assert_eq!(decode_string_list(encoded.as_slice()).unwrap(), values);

        let json = serde_json::to_vec(&values).unwrap();
        assert!(decode_string_list(&json).is_err(), "JSON PK arrays are not a storage format");
    }
}
