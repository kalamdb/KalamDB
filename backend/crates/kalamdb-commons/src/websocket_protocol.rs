use serde::{Deserialize, Serialize};

/// `Sec-WebSocket-Protocol` prefix used to carry a JWT on browser upgrades.
///
/// Browser `WebSocket` cannot set `Authorization`, so clients send
/// `kalamdb.jwt.<token>` as a subprotocol. The server echoes it and authenticates
/// during the HTTP upgrade, avoiding an extra Authenticate round-trip.
pub const WS_JWT_SUBPROTOCOL_PREFIX: &str = "kalamdb.jwt.";

/// Build a JWT WebSocket subprotocol when `token` is a valid HTTP token.
pub fn jwt_websocket_subprotocol(token: &str) -> Option<String> {
    if !is_http_token(token) {
        return None;
    }
    let mut protocol = String::with_capacity(WS_JWT_SUBPROTOCOL_PREFIX.len() + token.len());
    protocol.push_str(WS_JWT_SUBPROTOCOL_PREFIX);
    protocol.push_str(token);
    Some(protocol)
}

/// Extract a JWT from a `kalamdb.jwt.<token>` WebSocket subprotocol.
pub fn jwt_from_websocket_subprotocol(protocol: &str) -> Option<&str> {
    let token = protocol.strip_prefix(WS_JWT_SUBPROTOCOL_PREFIX)?;
    is_http_token(token).then_some(token)
}

/// RFC 7230 `tchar` — required for `Sec-WebSocket-Protocol` values.
fn is_http_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            matches!(
                byte,
                b'0'..=b'9'
                    | b'A'..=b'Z'
                    | b'a'..=b'z'
                    | b'!'
                    | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            )
        })
}

/// Wire-format serialization type negotiated during authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializationType {
    /// JSON text frames (default, backward-compatible).
    #[default]
    Json,
    /// MessagePack binary frames.
    #[serde(rename = "msgpack")]
    MessagePack,
}

/// Wire-format compression negotiated during authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionType {
    /// No compression.
    None,
    /// Gzip compression for payloads above threshold (default).
    #[default]
    Gzip,
}

/// Protocol options negotiated once per connection during authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolOptions {
    /// Serialization format for messages after auth.
    pub serialization: SerializationType,
    /// Compression policy.
    pub compression: CompressionType,
}

impl Default for ProtocolOptions {
    fn default() -> Self {
        Self {
            serialization: SerializationType::Json,
            compression: CompressionType::Gzip,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialization_type_json_roundtrip() {
        let ser = SerializationType::Json;
        let json = serde_json::to_string(&ser).unwrap();
        assert_eq!(json, "\"json\"");
        let parsed: SerializationType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SerializationType::Json);
    }

    #[test]
    fn test_serialization_type_msgpack_roundtrip() {
        let ser = SerializationType::MessagePack;
        let json = serde_json::to_string(&ser).unwrap();
        assert_eq!(json, "\"msgpack\"");
        let parsed: SerializationType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SerializationType::MessagePack);
    }

    #[test]
    fn test_protocol_options_default() {
        let opts = ProtocolOptions::default();
        assert_eq!(opts.serialization, SerializationType::Json);
        assert_eq!(opts.compression, CompressionType::Gzip);
    }

    #[test]
    fn test_protocol_options_roundtrip() {
        let opts = ProtocolOptions {
            serialization: SerializationType::MessagePack,
            compression: CompressionType::None,
        };
        let json = serde_json::to_string(&opts).unwrap();
        let parsed: ProtocolOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, opts);
    }

    #[test]
    fn jwt_subprotocol_roundtrip() {
        let token = "eyJhbGciOiJIUzI1NiJ9.e30.abc_def-012";
        let protocol = jwt_websocket_subprotocol(token).expect("valid jwt token");
        assert_eq!(protocol, format!("{WS_JWT_SUBPROTOCOL_PREFIX}{token}"));
        assert_eq!(jwt_from_websocket_subprotocol(&protocol), Some(token));
    }

    #[test]
    fn jwt_subprotocol_rejects_invalid_http_tokens() {
        assert!(jwt_websocket_subprotocol("abc=def").is_none());
        assert!(jwt_websocket_subprotocol("abc def").is_none());
        assert!(jwt_from_websocket_subprotocol("kalamdb.jwt.abc=def").is_none());
        assert!(jwt_from_websocket_subprotocol("chat").is_none());
    }
}
