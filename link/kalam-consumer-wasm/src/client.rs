use link_common::consumer::{
    decode_ack_response, decode_consume_response, AckRequestContext, ConsumeRequestContext,
};
use wasm_bindgen::prelude::*;

use crate::helpers::{
    fetch_json_response, js_value_to_json_string, response_text, serialize_json_to_js_value,
    topic_request_error,
};

#[wasm_bindgen]
pub struct KalamConsumerClient {
    url: String,
}

#[wasm_bindgen]
impl KalamConsumerClient {
    #[wasm_bindgen(constructor)]
    pub fn new(url: String) -> Result<KalamConsumerClient, JsValue> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(JsValue::from_str("Base URL must start with http:// or https://"));
        }

        Ok(Self {
            url: url.trim_end_matches('/').to_string(),
        })
    }

    pub async fn consume(
        &self,
        auth_header: Option<String>,
        request: JsValue,
    ) -> Result<JsValue, JsValue> {
        let body = js_value_to_json_string(&request, "Consume request")?;
        let request_context: ConsumeRequestContext = serde_json::from_str(&body)
            .map_err(|error| JsValue::from_str(&format!("Invalid consume request: {}", error)))?;

        let response = fetch_json_response(
            &format!("{}/v1/api/topics/consume", self.url),
            &body,
            auth_header.as_deref(),
        )
        .await?;
        let status = response.status();
        let text = response_text(&response).await?;
        if !response.ok() {
            return Err(topic_request_error(status, &text, "Consume failed"));
        }

        let response = decode_consume_response(&text, &request_context)
            .map_err(|error| JsValue::from_str(&error))?;

        serialize_json_to_js_value(&response, "consume response")
    }

    pub async fn ack(
        &self,
        auth_header: Option<String>,
        request: JsValue,
    ) -> Result<JsValue, JsValue> {
        let body = js_value_to_json_string(&request, "Ack request")?;
        let request_context: AckRequestContext = serde_json::from_str(&body)
            .map_err(|error| JsValue::from_str(&format!("Invalid ack request: {}", error)))?;

        let response = fetch_json_response(
            &format!("{}/v1/api/topics/ack", self.url),
            &body,
            auth_header.as_deref(),
        )
        .await?;
        let status = response.status();
        let text = response_text(&response).await?;
        if !response.ok() {
            return Err(topic_request_error(status, &text, "Ack failed"));
        }

        let response = decode_ack_response(&text, request_context)
            .map_err(|error| JsValue::from_str(&error))?;

        serialize_json_to_js_value(&response, "ack response")
    }
}
