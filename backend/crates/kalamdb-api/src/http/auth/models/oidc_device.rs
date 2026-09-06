use serde::{Deserialize, Serialize};

use super::UserInfo;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcDeviceStartRequest {
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcDeviceStartResponse {
    pub device_session_id:         String,
    pub verification_uri:          String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub user_code:                 String,
    pub expires_in_seconds:        u64,
    pub interval_seconds:          u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcDevicePollRequest {
    #[serde(alias = "session_id")]
    pub device_session_id: String,
}

#[derive(Debug, Serialize)]
pub struct OidcDevicePollResponse {
    pub status:             OidcDevicePollStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_seconds:   Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type:         Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token:       Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at:         Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token:      Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user:               Option<UserInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_ui_access:    Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message:            Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OidcDevicePollStatus {
    Pending,
    SlowDown,
    Authorized,
    Denied,
    Expired,
    Failed,
}
