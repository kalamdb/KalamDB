//! Unified authentication module for HTTP and WebSocket handlers.

mod audit;
mod bearer;
mod bearer_session_cache;
mod password;
mod types;
mod wire;

use std::sync::Arc;

pub use audit::extract_user_id_for_audit;
use bearer::authenticate_bearer;
pub use bearer_session_cache::invalidate_bearer_sessions;
use kalamdb_commons::{models::ConnectionInfo, Role};
use once_cell::sync::Lazy;
use password::authenticate_user_password;
use tracing::Instrument;
pub use types::{AuthMethod, AuthRequest, AuthenticationResult};
pub use wire::{authenticate_wire_password, WireAuthResult, WirePasswordAuthRequest};

use crate::{
    errors::error::AuthResult,
    helpers::authorization_header::extract_bearer_token,
    models::context::AuthenticatedUser,
    providers::{jwt_auth, jwt_config},
    repository::user_repo::UserRepository,
    services::login_tracker::LoginTracker,
};

/// Cached login tracker instance.
static LOGIN_TRACKER: Lazy<LoginTracker> = Lazy::new(LoginTracker::new);

/// Initialize auth configuration from server settings.
pub fn init_auth_config(auth: &kalamdb_configs::AuthSettings) {
    let oidc_default_role = Role::from_str_opt(&auth.oidc.default_role).unwrap_or_else(|| {
        log::warn!(
            "Invalid auth.oidc.default_role '{}'; falling back to 'user'",
            auth.oidc.default_role
        );
        Role::User
    });

    jwt_config::init_jwt_config(
        &auth.jwt_secret,
        &auth.jwt_trusted_issuers,
        Some(&auth.oidc),
        auth.oidc.enabled && auth.oidc.auto_provision,
        oidc_default_role,
    );
}

/// Authenticate a request using the unified authentication flow.
pub async fn authenticate(
    request: AuthRequest,
    connection_info: &kalamdb_commons::models::ConnectionInfo,
    repo: &Arc<dyn UserRepository>,
) -> AuthResult<AuthenticationResult> {
    if let Some(user) = try_cached_auth_request(&request, connection_info) {
        return Ok(AuthenticationResult {
            user,
            method: AuthMethod::Bearer,
        });
    }

    let request_kind = match &request {
        AuthRequest::Header(_) => "header",
        AuthRequest::Credentials { .. } => "credentials",
        AuthRequest::Jwt { .. } => "jwt",
    };

    let span = tracing::info_span!(
        "auth.check",
        auth_request_kind = request_kind,
        is_localhost = connection_info.is_localhost(),
        role = tracing::field::Empty,
        user = tracing::field::Empty
    );

    async move {
        match request {
            AuthRequest::Header(header) => {
                authenticate_header(&header, connection_info, repo).await
            },
            AuthRequest::Credentials { user, password } => {
                authenticate_credentials(&user, &password, connection_info, repo).await
            },
            AuthRequest::Jwt { token } => {
                let user = authenticate_bearer(&token, connection_info, repo).await?;
                record_authenticated_span(&user);
                Ok(AuthenticationResult {
                    user,
                    method: AuthMethod::Bearer,
                })
            },
        }
    }
    .instrument(span)
    .await
}

/// Resolve a validated KalamDB refresh token to the user it represents.
pub async fn resolve_refresh_token_user(
    repo: &Arc<dyn UserRepository>,
    claims: &jwt_auth::JwtClaims,
    connection_info: &ConnectionInfo,
) -> AuthResult<AuthenticatedUser> {
    let config = jwt_config::get_jwt_config();
    bearer::resolve_internal_authenticated_user(repo, &config, claims, connection_info).await
}

async fn authenticate_header(
    auth_header: &str,
    connection_info: &kalamdb_commons::models::ConnectionInfo,
    repo: &Arc<dyn UserRepository>,
) -> AuthResult<AuthenticationResult> {
    let token = extract_bearer_token(auth_header)?;
    let user = authenticate_bearer(token, connection_info, repo).await?;
    record_authenticated_span(&user);

    Ok(AuthenticationResult {
        user,
        method: AuthMethod::Bearer,
    })
}

async fn authenticate_credentials(
    user_id_str: &str,
    password: &str,
    connection_info: &kalamdb_commons::models::ConnectionInfo,
    repo: &Arc<dyn UserRepository>,
) -> AuthResult<AuthenticationResult> {
    let user = authenticate_user_password(user_id_str, password, connection_info, repo).await?;
    Ok(AuthenticationResult {
        user,
        method: AuthMethod::Direct,
    })
}

fn record_authenticated_span(user: &AuthenticatedUser) {
    tracing::Span::current().record("role", format!("{:?}", user.role).as_str());
    tracing::Span::current().record("user", user.user_id.as_str());
}

/// Return a cached bearer identity when `auth_header` is a previously verified token.
pub fn try_cached_bearer_session(
    auth_header: &str,
    connection_info: &ConnectionInfo,
) -> Option<AuthenticatedUser> {
    let token = extract_bearer_token(auth_header).ok()?;
    bearer_session_cache::lookup_cached_bearer_session(token, connection_info)
}

fn try_cached_auth_request(
    request: &AuthRequest,
    connection_info: &ConnectionInfo,
) -> Option<AuthenticatedUser> {
    match request {
        AuthRequest::Header(header) => try_cached_bearer_session(header, connection_info),
        AuthRequest::Jwt { token } => {
            bearer_session_cache::lookup_cached_bearer_session(token, connection_info)
        },
        AuthRequest::Credentials { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::UserId;

    use super::*;

    #[test]
    fn test_auth_method_debug() {
        assert_eq!(format!("{:?}", AuthMethod::Basic), "Basic");
        assert_eq!(format!("{:?}", AuthMethod::Bearer), "Bearer");
        assert_eq!(format!("{:?}", AuthMethod::Direct), "Direct");
    }

    #[test]
    fn try_cached_bearer_session_returns_stored_identity() {
        use kalamdb_commons::{models::ConnectionInfo, AuthType, Role};

        use crate::models::context::AuthenticatedUser;

        bearer_session_cache::clear_bearer_session_cache();
        let user = AuthenticatedUser::with_auth_type(
            UserId::new("cache_header_user"),
            Role::Dba,
            AuthType::Password,
            None,
            None,
            1,
            2,
            ConnectionInfo::new(Some("10.0.0.1".to_string())),
        );
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as usize)
            .unwrap_or(0)
            .saturating_add(3600);
        bearer_session_cache::store_cached_bearer_session("header-token", &user, exp);

        let cached = try_cached_bearer_session("Bearer header-token", &user.connection_info)
            .expect("cache hit");
        assert_eq!(cached.user_id.as_str(), "cache_header_user");
        assert_eq!(cached.role, Role::Dba);
        assert!(try_cached_bearer_session("Bearer missing", &user.connection_info).is_none());
    }

    #[test]
    fn test_extract_user_id_from_credentials() {
        let request = AuthRequest::Credentials {
            user: "testuser".to_string(),
            password: "secret".to_string(),
        };
        assert_eq!(extract_user_id_for_audit(&request), UserId::from("testuser"));
    }

    #[test]
    fn test_extract_user_id_from_bearer_header() {
        let request = AuthRequest::Header("Bearer some.jwt.token".to_string());
        assert_eq!(extract_user_id_for_audit(&request), UserId::anonymous());
    }

    #[test]
    fn test_extract_user_id_from_jwt_with_sub() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"user_from_sub","exp":9999999999}"#);
        let signature = "fake_signature";

        let token = format!("{}.{}.{}", header, payload, signature);
        let request = AuthRequest::Jwt { token };
        assert_eq!(extract_user_id_for_audit(&request), UserId::from("user_from_sub"));
    }

    #[test]
    fn test_extract_user_id_from_invalid_jwt() {
        let request = AuthRequest::Jwt {
            token: "invalid_token".to_string(),
        };
        assert_eq!(extract_user_id_for_audit(&request), UserId::anonymous());
    }

    #[test]
    fn test_extract_user_id_from_bearer_header_with_jwt() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"bearer_user","exp":9999999999}"#);
        let signature = "fake_signature";

        let token = format!("{}.{}.{}", header, payload, signature);
        let request = AuthRequest::Header(format!("Bearer {}", token));
        assert_eq!(extract_user_id_for_audit(&request), UserId::from("bearer_user"));

        let request = AuthRequest::Header(format!("bearer {}", token));
        assert_eq!(extract_user_id_for_audit(&request), UserId::from("bearer_user"));

        let request = AuthRequest::Header(format!("Bearerish {}", token));
        assert_eq!(extract_user_id_for_audit(&request), UserId::anonymous());
    }

    #[cfg(feature = "websocket")]
    #[test]
    fn test_from_ws_auth_credentials_jwt() {
        use kalamdb_commons::websocket_auth::WsAuthCredentials;

        let ws_creds = WsAuthCredentials::Jwt {
            token: "my.jwt.token".to_string(),
        };

        let auth_request: AuthRequest = ws_creds.into();
        match auth_request {
            AuthRequest::Jwt { token } => {
                assert_eq!(token, "my.jwt.token");
            },
            _ => panic!("Expected Jwt variant"),
        }
    }
}
