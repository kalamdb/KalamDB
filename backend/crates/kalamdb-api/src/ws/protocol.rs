use actix_web::{http::header, HttpRequest, HttpResponse};
use actix_ws::ProtocolError;
use kalamdb_auth::{extract_bearer_token, AuthRequest};
use kalamdb_commons::websocket::{
    jwt_from_websocket_subprotocol, CompressionType, ProtocolOptions, SerializationType,
};

use super::context::UpgradeAuth;

pub(super) fn parse_protocol_from_query(query: &str) -> ProtocolOptions {
    let mut protocol = ProtocolOptions::default();
    for kv in query.split('&') {
        if let Some((key, value)) = kv.split_once('=') {
            match key {
                "serialization" => {
                    if value.eq_ignore_ascii_case("msgpack") {
                        protocol.serialization = SerializationType::MessagePack;
                    }
                },
                "compression" => {
                    if value.eq_ignore_ascii_case("none") {
                        protocol.compression = CompressionType::None;
                    }
                },
                _ => {},
            }
        }
    }
    protocol
}

pub(super) fn compression_enabled_from_query(req: &HttpRequest) -> bool {
    !req.query_string()
        .split('&')
        .any(|kv| kv.eq_ignore_ascii_case("compress=false"))
}

pub(super) fn validate_origin(
    req: &HttpRequest,
    app_context: &kalamdb_core::app_context::AppContext,
) -> Result<(), HttpResponse> {
    let config = app_context.config();
    let allowed_origins = &config.security.cors.allowed_origins;

    if allowed_origins.is_empty() || allowed_origins.contains(&"*".to_string()) {
        return Ok(());
    }

    if let Some(origin) = req.headers().get("Origin") {
        if let Ok(origin_str) = origin.to_str() {
            if allowed_origins.iter().any(|allowed| allowed == origin_str) {
                return Ok(());
            }
            log::warn!("WebSocket connection rejected: invalid origin '{}'", origin_str);
            return Err(HttpResponse::Forbidden().body("Origin not allowed"));
        }
    }

    if config.security.strict_ws_origin_check {
        log::warn!("WebSocket connection rejected: missing Origin header");
        return Err(HttpResponse::Forbidden().body("Origin header required"));
    }

    Ok(())
}

pub(super) fn parse_upgrade_auth(req: &HttpRequest) -> Option<UpgradeAuth> {
    let protocol = parse_protocol_from_query(req.query_string());
    if let Some(token) = bearer_from_authorization(req) {
        return Some(UpgradeAuth {
            auth_request: AuthRequest::Jwt { token },
            protocol,
            echo_subprotocol: None,
        });
    }

    jwt_from_sec_websocket_protocol(req).map(|(token, offered)| UpgradeAuth {
        auth_request: AuthRequest::Jwt { token },
        protocol,
        echo_subprotocol: Some(offered),
    })
}

fn bearer_from_authorization(req: &HttpRequest) -> Option<String> {
    let auth_header = req.headers().get("Authorization")?;
    let auth_str = auth_header.to_str().ok()?;
    extract_bearer_token(auth_str).ok().map(str::to_string)
}

fn jwt_from_sec_websocket_protocol(req: &HttpRequest) -> Option<(String, String)> {
    for value in req.headers().get_all(header::SEC_WEBSOCKET_PROTOCOL) {
        let Ok(offered) = value.to_str() else {
            continue;
        };
        for protocol in offered.split(',').map(str::trim) {
            if let Some(token) = jwt_from_websocket_subprotocol(protocol) {
                return Some((token.to_string(), protocol.to_string()));
            }
        }
    }
    None
}

pub(super) fn is_expected_ws_disconnect(error: &ProtocolError) -> bool {
    match error {
        ProtocolError::Io(io_err) => {
            use std::io::ErrorKind::*;
            if matches!(
                io_err.kind(),
                BrokenPipe | ConnectionReset | ConnectionAborted | UnexpectedEof
            ) {
                return true;
            }

            let msg = io_err.to_string().to_ascii_lowercase();
            msg.contains("eof")
                || msg.contains("connection reset")
                || msg.contains("broken pipe")
                || msg.contains("connection aborted")
                || msg.contains("payload reached eof")
                || msg.contains("connection closed")
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use actix_web::{
        http::{header, StatusCode},
        test::TestRequest,
    };
    use kalamdb_auth::AuthRequest;
    use kalamdb_commons::{
        websocket::{jwt_websocket_subprotocol, CompressionType, SerializationType},
        NodeId,
    };
    use kalamdb_configs::ServerConfig;
    use kalamdb_core::app_context::AppContext;
    use kalamdb_store::test_utils::InMemoryBackend;
    use uuid::Uuid;

    use super::{parse_protocol_from_query, parse_upgrade_auth, validate_origin};

    fn test_app_context_with_origin_policy(
        cors_allowed_origins: Vec<String>,
        strict_ws_origin_check: bool,
    ) -> Arc<AppContext> {
        let mut config = ServerConfig::default();
        config.security.cors.allowed_origins = cors_allowed_origins;
        config.security.strict_ws_origin_check = strict_ws_origin_check;

        AppContext::init_test(
            Arc::new(InMemoryBackend::new()),
            NodeId::new(91),
            format!("/tmp/kalamdb-ws-origin-{}", Uuid::new_v4()),
            config,
        )
    }

    #[test]
    fn parse_protocol_defaults_when_empty() {
        let proto = parse_protocol_from_query("");
        assert_eq!(proto.serialization, SerializationType::Json);
        assert_eq!(proto.compression, CompressionType::Gzip);
    }

    #[test]
    fn parse_protocol_msgpack_serialization() {
        let proto = parse_protocol_from_query("serialization=msgpack");
        assert_eq!(proto.serialization, SerializationType::MessagePack);
        assert_eq!(proto.compression, CompressionType::Gzip);
    }

    #[test]
    fn parse_protocol_compression_none() {
        let proto = parse_protocol_from_query("compression=none");
        assert_eq!(proto.serialization, SerializationType::Json);
        assert_eq!(proto.compression, CompressionType::None);
    }

    #[test]
    fn parse_protocol_both_options() {
        let proto = parse_protocol_from_query("serialization=msgpack&compression=none");
        assert_eq!(proto.serialization, SerializationType::MessagePack);
        assert_eq!(proto.compression, CompressionType::None);
    }

    #[test]
    fn parse_protocol_mixed_with_compress_false() {
        let proto = parse_protocol_from_query("compress=false&serialization=msgpack");
        assert_eq!(proto.serialization, SerializationType::MessagePack);
        assert_eq!(proto.compression, CompressionType::Gzip);
    }

    #[test]
    fn parse_protocol_case_insensitive() {
        let proto = parse_protocol_from_query("serialization=MSGPACK&compression=NONE");
        assert_eq!(proto.serialization, SerializationType::MessagePack);
        assert_eq!(proto.compression, CompressionType::None);
    }

    #[test]
    fn parse_protocol_unknown_values_keep_defaults() {
        let proto = parse_protocol_from_query("serialization=avro&compression=lz4");
        assert_eq!(proto.serialization, SerializationType::Json);
        assert_eq!(proto.compression, CompressionType::Gzip);
    }

    #[test]
    fn parse_upgrade_auth_reads_authorization_bearer() {
        let request = TestRequest::default()
            .insert_header(("Authorization", "Bearer header-token"))
            .to_http_request();
        let auth = parse_upgrade_auth(&request).expect("bearer header should authenticate");
        match auth.auth_request {
            AuthRequest::Jwt { token } => assert_eq!(token, "header-token"),
            other => panic!("expected jwt, got {other:?}"),
        }
        assert!(auth.echo_subprotocol.is_none());
    }

    #[test]
    fn parse_upgrade_auth_reads_jwt_subprotocol() {
        let protocol = jwt_websocket_subprotocol("ws-token").unwrap();
        let request = TestRequest::default()
            .insert_header((header::SEC_WEBSOCKET_PROTOCOL, protocol.as_str()))
            .to_http_request();
        let auth = parse_upgrade_auth(&request).expect("jwt subprotocol should authenticate");
        match auth.auth_request {
            AuthRequest::Jwt { token } => assert_eq!(token, "ws-token"),
            other => panic!("expected jwt, got {other:?}"),
        }
        assert_eq!(auth.echo_subprotocol.as_deref(), Some(protocol.as_str()));
    }

    #[test]
    fn parse_upgrade_auth_prefers_authorization_over_subprotocol() {
        let protocol = jwt_websocket_subprotocol("protocol-token").unwrap();
        let request = TestRequest::default()
            .insert_header(("Authorization", "Bearer header-token"))
            .insert_header((header::SEC_WEBSOCKET_PROTOCOL, protocol.as_str()))
            .to_http_request();
        let auth = parse_upgrade_auth(&request).expect("authorization should win");
        match auth.auth_request {
            AuthRequest::Jwt { token } => assert_eq!(token, "header-token"),
            other => panic!("expected jwt, got {other:?}"),
        }
        assert!(auth.echo_subprotocol.is_none());
    }

    #[actix_rt::test]
    async fn validate_origin_rejects_unlisted_origin() {
        let app_context = test_app_context_with_origin_policy(
            vec!["https://admin.example.com".to_string()],
            false,
        );
        let request = TestRequest::default()
            .insert_header(("Origin", "https://evil.example.com"))
            .to_http_request();

        let response = validate_origin(&request, app_context.as_ref())
            .expect_err("unexpectedly allowed an unlisted origin");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[actix_rt::test]
    async fn validate_origin_rejects_missing_origin_when_strict() {
        let app_context = test_app_context_with_origin_policy(
            vec!["https://admin.example.com".to_string()],
            true,
        );
        let request = TestRequest::default().to_http_request();

        let response = validate_origin(&request, app_context.as_ref())
            .expect_err("strict origin checking should require Origin header");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[actix_rt::test]
    async fn validate_origin_allows_configured_cors_origin() {
        let app_context =
            test_app_context_with_origin_policy(vec!["https://app.example.com".to_string()], true);
        let request = TestRequest::default()
            .insert_header(("Origin", "https://app.example.com"))
            .to_http_request();

        validate_origin(&request, app_context.as_ref())
            .expect("configured CORS origin should also be allowed for WebSocket upgrades");
    }
}
