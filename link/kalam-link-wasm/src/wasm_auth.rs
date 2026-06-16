use base64::{engine::general_purpose, Engine as _};
use wasm_bindgen::prelude::JsValue;
use wasm_bindgen_futures::JsFuture;

use link_common::models::{ClientMessage, ProtocolOptions, WsAuthCredentials};

/// Authentication provider for WASM clients
///
/// Supports three authentication modes:
/// - Basic: HTTP Basic Auth with username/password (HTTP only)
/// - Jwt: Bearer token authentication with JWT (required for WebSocket)
/// - None: No authentication (localhost bypass)
#[derive(Clone)]
pub(crate) enum WasmAuthProvider {
    /// HTTP Basic Authentication (username/password)
    Basic { username: String, password: String },
    /// JWT Token Authentication (Bearer token)
    Jwt { token: String },
    /// No authentication (for localhost bypass)
    None,
}

impl WasmAuthProvider {
    /// Get the Authorization header value for HTTP requests
    pub(crate) fn to_http_header(&self) -> Option<String> {
        match self {
            WasmAuthProvider::Basic { username, password } => {
                let credentials = format!("{}:{}", username, password);
                let encoded = general_purpose::STANDARD.encode(credentials.as_bytes());
                Some(format!("Basic {}", encoded))
            },
            WasmAuthProvider::Jwt { token } => Some(format!("Bearer {}", token)),
            WasmAuthProvider::None => None,
        }
    }

    /// Get the WebSocket authentication message using unified WsAuthCredentials
    pub(crate) fn to_ws_auth_message(&self, protocol: ProtocolOptions) -> Option<ClientMessage> {
        match self {
            WasmAuthProvider::Basic { .. } => None,
            WasmAuthProvider::Jwt { token } => Some(ClientMessage::Authenticate {
                credentials: WsAuthCredentials::Jwt {
                    token: token.clone(),
                },
                protocol,
            }),
            WasmAuthProvider::None => None,
        }
    }
}

pub(crate) async fn resolve_auth_provider(
    auth_provider_cb: Option<js_sys::Function>,
    fallback: WasmAuthProvider,
) -> Result<WasmAuthProvider, JsValue> {
    let Some(cb) = auth_provider_cb else {
        return Ok(fallback);
    };

    let result = JsFuture::from(js_sys::Promise::resolve(
        &cb.call0(&JsValue::NULL)
            .map_err(|error| JsValue::from_str(&format!("authProvider threw: {:?}", error)))?,
    ))
    .await?;

    auth_from_js_value(result)
}

fn auth_from_js_value(value: JsValue) -> Result<WasmAuthProvider, JsValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(WasmAuthProvider::None);
    }

    let jwt_obj = js_sys::Reflect::get(&value, &"jwt".into()).ok();
    if let Some(jwt) = jwt_obj.filter(|jwt| !jwt.is_undefined() && !jwt.is_null()) {
        let token = js_sys::Reflect::get(&jwt, &"token".into())
            .ok()
            .and_then(|token| token.as_string())
            .ok_or_else(|| {
                JsValue::from_str("authProvider result must have shape { jwt: { token: string } }")
            })?;
        return Ok(WasmAuthProvider::Jwt { token });
    }

    Ok(WasmAuthProvider::None)
}
