use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthLoginOptionsResponse {
    pub local: LocalLoginOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oidc:  Option<OidcLoginOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalLoginOptions {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcLoginOptions {
    pub enabled: bool,
    pub display_name: String,
    pub issuer: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_authorization_endpoint: Option<String>,
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_flow: Option<OidcDeviceFlowOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcDeviceFlowOptions {
    pub direct_supported:              bool,
    pub broker_supported:              bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_authorization_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_start_endpoint:         Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_poll_endpoint:          Option<String>,
}
