use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;

use crate::{
    consumer::models::{AckResponse, ConsumeMessage, ConsumeResponse, TopicOp},
    models::{RowData, UserId},
};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsumeRequestContext {
    #[serde(default)]
    pub topic_id:     String,
    #[serde(default)]
    pub group_id:     String,
    #[serde(default)]
    pub partition_id: u32,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct AckRequestContext {
    #[serde(default)]
    pub upto_offset: u64,
}

#[derive(Debug, Deserialize)]
struct RawAckResponse {
    success:             Option<bool>,
    acknowledged_offset: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawConsumeResponse {
    #[serde(default)]
    messages:    Vec<RawConsumeMessage>,
    #[serde(default)]
    next_offset: u64,
    #[serde(default)]
    has_more:    bool,
}

#[derive(Debug, Deserialize)]
struct RawConsumeMessage {
    #[serde(default, alias = "message_id")]
    key:          Option<String>,
    #[serde(default)]
    op:           Option<TopicOp>,
    #[serde(default, rename = "timestamp_ms", alias = "ts")]
    timestamp_ms: Option<u64>,
    #[serde(default)]
    offset:       u64,
    partition_id: Option<u32>,
    topic_id:     Option<String>,
    #[serde(default, alias = "username", alias = "user_id")]
    user:         Option<UserId>,
    #[serde(default, alias = "value")]
    payload:      Option<serde_json::Value>,
}

pub fn decode_consume_response(
    text: &str,
    request_context: &ConsumeRequestContext,
) -> Result<ConsumeResponse, String> {
    let raw: RawConsumeResponse = serde_json::from_str(text)
        .map_err(|error| format!("Failed to parse consume response: {}", error))?;
    let messages = raw
        .messages
        .into_iter()
        .map(|message| decode_consume_message(message, request_context))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ConsumeResponse {
        messages,
        next_offset: raw.next_offset,
        has_more: raw.has_more,
    })
}

pub fn decode_ack_response(
    text: &str,
    request_context: AckRequestContext,
) -> Result<AckResponse, String> {
    let raw: RawAckResponse = serde_json::from_str(text)
        .map_err(|error| format!("Failed to parse ack response: {}", error))?;

    Ok(AckResponse {
        success:             raw.success.unwrap_or(true),
        acknowledged_offset: raw.acknowledged_offset.unwrap_or(request_context.upto_offset),
    })
}

fn decode_consume_message(
    raw: RawConsumeMessage,
    request_context: &ConsumeRequestContext,
) -> Result<ConsumeMessage, String> {
    let user = raw.user.ok_or_else(|| {
        "Consume response message is missing required user metadata; upgrade the server or \
         republish the topic event with a user id"
            .to_string()
    })?;

    Ok(ConsumeMessage {
        key: raw.key,
        op: raw.op,
        timestamp_ms: raw.timestamp_ms,
        offset: raw.offset,
        partition_id: raw.partition_id.unwrap_or(request_context.partition_id),
        topic: raw.topic_id.unwrap_or_else(|| request_context.topic_id.clone()),
        group_id: request_context.group_id.clone(),
        user,
        payload: decode_payload_value(raw.payload),
    })
}

fn decode_payload_value(payload: Option<serde_json::Value>) -> RowData {
    let value = match payload {
        Some(serde_json::Value::String(payload)) => match STANDARD.decode(payload.as_bytes()) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::String(payload))
            },
            Err(_) => serde_json::Value::String(payload),
        },
        Some(value @ serde_json::Value::Object(_)) => value,
        Some(other) => other,
        None => serde_json::Value::Null,
    };

    serde_json::from_value(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        decode_ack_response, decode_consume_response, AckRequestContext, ConsumeRequestContext,
    };

    #[test]
    fn decode_consume_response_requires_user_metadata() {
        let request_context = ConsumeRequestContext {
            topic_id:     "orders".to_string(),
            group_id:     "billing".to_string(),
            partition_id: 0,
        };

        let error = decode_consume_response(
            r#"{
                "messages": [
                    {
                        "topic_id": "orders",
                        "partition_id": 0,
                        "offset": 9,
                        "payload": {"id": 9}
                    }
                ],
                "next_offset": 10,
                "has_more": false
            }"#,
            &request_context,
        )
        .unwrap_err();

        assert!(error.contains("missing required user metadata"));
    }

    #[test]
    fn decode_consume_response_accepts_user_id_alias() {
        let request_context = ConsumeRequestContext {
            topic_id:     "orders".to_string(),
            group_id:     "billing".to_string(),
            partition_id: 0,
        };

        let response = decode_consume_response(
            r#"{
                "messages": [
                    {
                        "topic_id": "orders",
                        "partition_id": 0,
                        "offset": 9,
                        "user_id": "producer-42",
                        "payload": {"id": 9}
                    }
                ],
                "next_offset": 10,
                "has_more": false
            }"#,
            &request_context,
        )
        .expect("consume response with user_id alias should decode");

        assert_eq!(response.messages.len(), 1);
        assert_eq!(response.messages[0].user.as_str(), "producer-42");
    }

    #[test]
    fn decode_ack_response_defaults_missing_offset_from_request() {
        let response =
            decode_ack_response(r#"{"success":true}"#, AckRequestContext { upto_offset: 42 })
                .expect("ack response should decode");

        assert!(response.success);
        assert_eq!(response.acknowledged_offset, 42);
    }
}
