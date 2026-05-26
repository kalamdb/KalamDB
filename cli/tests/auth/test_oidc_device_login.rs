use kalam_cli::session::{
    auth_options::{
        OidcDevicePollRequest, OidcDevicePollResponse, OidcDevicePollStatus,
        OidcDeviceStartResponse,
    },
    oidc_device::selected_device_scopes,
};
use serde_json::json;

#[test]
fn oidc_direct_device_scopes_leave_openid_to_openidconnect() {
    let scopes = selected_device_scopes(&[
        "openid".to_string(),
        "email".to_string(),
        "profile".to_string(),
    ]);
    let scopes: Vec<_> = scopes.iter().map(|scope| scope.as_ref().to_string()).collect();

    assert_eq!(scopes, vec!["email".to_string(), "profile".to_string()]);
}

#[test]
fn oidc_broker_device_contract_accepts_session_and_interval_aliases() {
    let start: OidcDeviceStartResponse = serde_json::from_value(json!({
        "session_id": "broker-session-1",
        "verification_uri": "https://issuer.example/device",
        "verification_uri_complete": "https://issuer.example/device?user_code=ABCD-EFGH",
        "user_code": "ABCD-EFGH",
        "expires_in": 600,
        "interval": 5
    }))
    .expect("broker start aliases should deserialize");

    assert_eq!(start.device_session_id, "broker-session-1");
    assert_eq!(start.expires_in_seconds, 600);
    assert_eq!(start.interval_seconds, 5);

    let poll_request = serde_json::to_value(OidcDevicePollRequest {
        device_session_id: start.device_session_id,
    })
    .expect("poll request should serialize");
    assert_eq!(poll_request, json!({ "device_session_id": "broker-session-1" }));

    let poll: OidcDevicePollResponse = serde_json::from_value(json!({
        "status": "slow_down",
        "message": "poll less frequently"
    }))
    .expect("broker poll status should deserialize");
    assert_eq!(poll.status, OidcDevicePollStatus::SlowDown);
    assert_eq!(poll.message.as_deref(), Some("poll less frequently"));
}
