//! Actix-web extractor for automatic authentication.
//!
//! This module provides `FromRequest` implementations for authentication,
//! allowing handlers to receive authenticated users as function parameters.
//!
//! # Setup
//!
//! The `Arc<dyn UserRepository>` must be registered as app data:
//!
//! ```rust,ignore
//! App::new()
//!     .app_data(web::Data::new(user_repo.clone()))
//!     .service(my_handler)
//! ```
//!
//! # Example
//!
//!
//! // Simple case - just the user
//! #[post("/api")]
//! async fn simple_handler(user: AuthenticatedUser) -> impl Responder {
//!     // user is guaranteed to be authenticated
//! }
//!
//! // Full auth info including method and connection info (for audit logging)
//! #[post("/sql")]
//! async fn sql_handler(session: AuthSession) -> impl Responder {
//!     // Access user via session.user
//!     // Access auth method via session.method (Bearer)
//!     // Access connection info via session.connection_info
//! }
//!
//! ```

use std::{fmt, future::Future, pin::Pin, sync::Arc};

use actix_web::{dev::Payload, http::StatusCode, FromRequest, HttpRequest, ResponseError};
use jsonwebtoken::Algorithm;
use kalamdb_commons::{models::ConnectionInfo, AuthType, Role, UserId};

use crate::{
    errors::error::AuthError,
    helpers::authorization_header::extract_bearer_token,
    helpers::ip_extractor::extract_client_ip_secure,
    providers::{jwt_auth, jwt_config::JwtConfig},
    repository::user_repo::UserRepository,
    services::unified::{authenticate, AuthRequest},
};

/// Error type for authentication extraction.
///
/// Wraps `AuthError` and implements `ResponseError` for automatic
/// HTTP error responses.
#[derive(Debug)]
pub struct AuthExtractError {
    inner: AuthError,
    /// Time taken to process the request (for response payload)
    took_ms: f64,
}

impl AuthExtractError {
    /// Create a new extraction error from an `AuthError`.
    pub fn new(error: AuthError, took_ms: f64) -> Self {
        Self {
            inner: error,
            took_ms,
        }
    }

    /// Get the underlying auth error.
    pub fn inner(&self) -> &AuthError {
        &self.inner
    }

    /// Get the error code for API responses.
    pub fn error_code(&self) -> &'static str {
        match &self.inner {
            AuthError::MissingAuthorization(_) => "MISSING_AUTHORIZATION",
            AuthError::MalformedAuthorization(_) => "MALFORMED_AUTHORIZATION",
            AuthError::InvalidCredentials(_) => "INVALID_CREDENTIALS",
            AuthError::RemoteAccessDenied(_) => "REMOTE_ACCESS_DENIED",
            AuthError::UserNotFound(_) => "USER_NOT_FOUND",
            AuthError::DatabaseError(_) => "DATABASE_ERROR",
            AuthError::TokenExpired => "TOKEN_EXPIRED",
            AuthError::InvalidSignature => "INVALID_SIGNATURE",
            AuthError::UntrustedIssuer(_) => "UNTRUSTED_ISSUER",
            AuthError::MissingClaim(_) => "MISSING_CLAIM",
            AuthError::InsufficientPermissions(_) => "INSUFFICIENT_PERMISSIONS",
            _ => "AUTHENTICATION_ERROR",
        }
    }

    fn public_error_code(&self) -> &'static str {
        match &self.inner {
            AuthError::MissingAuthorization(_) => "MISSING_AUTHORIZATION",
            AuthError::TokenExpired => "TOKEN_EXPIRED",
            AuthError::RemoteAccessDenied(_) => "REMOTE_ACCESS_DENIED",
            AuthError::InsufficientPermissions(_) => "INSUFFICIENT_PERMISSIONS",
            AuthError::SetupRequired(_) => "SETUP_REQUIRED",
            AuthError::AccountLocked(_) => "ACCOUNT_LOCKED",
            AuthError::MalformedAuthorization(_)
            | AuthError::InvalidCredentials(_)
            | AuthError::UserNotFound(_)
            | AuthError::UserDeleted
            | AuthError::InvalidSignature
            | AuthError::UntrustedIssuer(_)
            | AuthError::MissingClaim(_)
            | AuthError::WeakPassword(_)
            | AuthError::AuthenticationFailed(_) => "INVALID_CREDENTIALS",
            AuthError::DatabaseError(_) | AuthError::HashingError(_) => "AUTHENTICATION_ERROR",
        }
    }

    fn public_message(&self) -> &'static str {
        match &self.inner {
            AuthError::MissingAuthorization(_) => "Authentication required",
            AuthError::MalformedAuthorization(_)
            | AuthError::InvalidCredentials(_)
            | AuthError::UserNotFound(_)
            | AuthError::TokenExpired
            | AuthError::InvalidSignature
            | AuthError::UntrustedIssuer(_)
            | AuthError::MissingClaim(_)
            | AuthError::AuthenticationFailed(_) => "Invalid credentials",
            AuthError::RemoteAccessDenied(_) => "Access denied",
            AuthError::InsufficientPermissions(_) => "Insufficient permissions",
            AuthError::DatabaseError(_) | AuthError::HashingError(_) => "Authentication failed",
            AuthError::SetupRequired(_) => "Server requires initial setup",
            AuthError::AccountLocked(_) => "Account locked",
            AuthError::WeakPassword(_) => "Invalid credentials",
            AuthError::UserDeleted => "Invalid credentials",
        }
    }
}

impl fmt::Display for AuthExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl ResponseError for AuthExtractError {
    fn status_code(&self) -> StatusCode {
        match &self.inner {
            AuthError::MissingAuthorization(_) => StatusCode::UNAUTHORIZED,
            // Malformed tokens are an authentication failure, not a protocol
            // error.  Returning 401 avoids leaking whether a token was well-
            // formed vs simply wrong (OWASP information-disclosure mitigation).
            AuthError::MalformedAuthorization(_) => StatusCode::UNAUTHORIZED,
            AuthError::InvalidCredentials(_) => StatusCode::UNAUTHORIZED,
            AuthError::RemoteAccessDenied(_) => StatusCode::FORBIDDEN,
            AuthError::UserNotFound(_) => StatusCode::UNAUTHORIZED,
            AuthError::TokenExpired => StatusCode::UNAUTHORIZED,
            AuthError::InvalidSignature => StatusCode::UNAUTHORIZED,
            AuthError::UntrustedIssuer(_) => StatusCode::UNAUTHORIZED,
            AuthError::MissingClaim(_) => StatusCode::UNAUTHORIZED,
            AuthError::InsufficientPermissions(_) => StatusCode::FORBIDDEN,
            AuthError::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::UNAUTHORIZED,
        }
    }

    fn error_response(&self) -> actix_web::HttpResponse {
        let body = serde_json::json!({
            "status": "error",
            "error": {
                "code": self.public_error_code(),
                "message": self.public_message()
            },
            "results": [],
            "took": self.took_ms
        });

        actix_web::HttpResponse::build(self.status_code())
            .content_type("application/json")
            .json(body)
    }
}

impl From<AuthError> for AuthExtractError {
    fn from(error: AuthError) -> Self {
        Self {
            inner: error,
            took_ms: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_web::body::to_bytes;
    use serde_json::Value;

    use super::*;
    use crate::providers::jwt_auth::{
        create_and_sign_refresh_token_with_auth_type, create_and_sign_token_with_auth_type,
    };

    #[test]
    fn test_auth_extract_error_codes() {
        let err = AuthExtractError::new(AuthError::MissingAuthorization("test".to_string()), 10.0);
        assert_eq!(err.error_code(), "MISSING_AUTHORIZATION");
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);

        let err =
            AuthExtractError::new(AuthError::MalformedAuthorization("test".to_string()), 10.0);
        assert_eq!(err.error_code(), "MALFORMED_AUTHORIZATION");
        // Malformed tokens return 401 (unauthorized) to avoid leaking whether a
        // request was well-formed vs. simply wrong (OWASP AA05 guidance).
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);

        let err = AuthExtractError::new(AuthError::TokenExpired, 10.0);
        assert_eq!(err.error_code(), "TOKEN_EXPIRED");
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);

        let err =
            AuthExtractError::new(AuthError::InsufficientPermissions("test".to_string()), 10.0);
        assert_eq!(err.error_code(), "INSUFFICIENT_PERMISSIONS");
        assert_eq!(err.status_code(), StatusCode::FORBIDDEN);
    }

    async fn error_body(err: AuthExtractError) -> Value {
        let response = err.error_response();
        let body = to_bytes(response.into_body())
            .await
            .expect("extractor error body should be readable");
        serde_json::from_slice(&body).expect("extractor error body should be valid JSON")
    }

    #[tokio::test]
    async fn test_sensitive_extractor_errors_are_redacted() {
        let body = error_body(AuthExtractError::new(
            AuthError::UntrustedIssuer("evil.com".to_string()),
            10.0,
        ))
        .await;
        assert_eq!(body["error"]["code"], "INVALID_CREDENTIALS");
        assert_eq!(body["error"]["message"], "Invalid credentials");

        let body = error_body(AuthExtractError::new(AuthError::TokenExpired, 10.0)).await;
        assert_eq!(body["error"]["code"], "TOKEN_EXPIRED");
        assert_eq!(body["error"]["message"], "Invalid credentials");

        let body =
            error_body(AuthExtractError::new(AuthError::MissingClaim("sub".to_string()), 10.0))
                .await;
        assert_eq!(body["error"]["code"], "INVALID_CREDENTIALS");
        assert_eq!(body["error"]["message"], "Invalid credentials");
    }

    #[tokio::test]
    async fn test_extractor_errors_keep_safe_public_messages() {
        let body = error_body(AuthExtractError::new(
            AuthError::MissingAuthorization("header missing".to_string()),
            10.0,
        ))
        .await;
        assert_eq!(body["error"]["code"], "MISSING_AUTHORIZATION");
        assert_eq!(body["error"]["message"], "Authentication required");

        let body = error_body(AuthExtractError::new(
            AuthError::InsufficientPermissions("role mismatch".to_string()),
            10.0,
        ))
        .await;
        assert_eq!(body["error"]["code"], "INSUFFICIENT_PERMISSIONS");
        assert_eq!(body["error"]["message"], "Insufficient permissions");

        let body = error_body(AuthExtractError::new(AuthError::InvalidSignature, 10.0)).await;
        assert_eq!(body["error"]["code"], "INVALID_CREDENTIALS");
        assert_eq!(body["error"]["message"], "Invalid credentials");
    }

    #[test]
    fn stateless_fast_path_accepts_internal_oidc_user_access_token() {
        let config = JwtConfig::for_tests(true, Role::User);
        let (token, _) = create_and_sign_token_with_auth_type(
            &UserId::new("oidc_user"),
            &Role::User,
            Some("oidc@example.com"),
            None,
            &config.secret,
            AuthType::Oidc,
        )
        .expect("test token should be created");

        let session = try_build_stateless_oidc_user_session(
            &format!("Bearer {token}"),
            &ConnectionInfo::new(Some("127.0.0.1".to_string())),
            None,
            &config,
        )
        .expect("fast path should succeed")
        .expect("oidc user token should use fast path");

        assert_eq!(session.user_id().as_str(), "oidc_user");
        assert_eq!(session.role(), Role::User);
        assert_eq!(session.auth_method(), kalamdb_session::AuthMethod::Bearer);
    }

    #[test]
    fn stateless_fast_path_skips_non_oidc_or_non_user_tokens() {
        let config = JwtConfig::for_tests(true, Role::User);
        let (password_token, _) = create_and_sign_token_with_auth_type(
            &UserId::new("password_user"),
            &Role::User,
            Some("password@example.com"),
            None,
            &config.secret,
            AuthType::Password,
        )
        .expect("password token should be created");
        let (dba_token, _) = create_and_sign_token_with_auth_type(
            &UserId::new("dba_user"),
            &Role::Dba,
            Some("dba@example.com"),
            None,
            &config.secret,
            AuthType::Oidc,
        )
        .expect("dba token should be created");

        assert!(try_build_stateless_oidc_user_session(
            &format!("Bearer {password_token}"),
            &ConnectionInfo::new(Some("127.0.0.1".to_string())),
            None,
            &config,
        )
        .expect("password token should parse")
        .is_none());

        assert!(try_build_stateless_oidc_user_session(
            &format!("Bearer {dba_token}"),
            &ConnectionInfo::new(Some("127.0.0.1".to_string())),
            None,
            &config,
        )
        .expect("dba token should parse")
        .is_none());
    }

    #[test]
    fn stateless_fast_path_rejects_refresh_tokens() {
        let config = JwtConfig::for_tests(true, Role::User);
        let (refresh_token, _) = create_and_sign_refresh_token_with_auth_type(
            &UserId::new("oidc_user"),
            &Role::User,
            Some("oidc@example.com"),
            None,
            &config.secret,
            AuthType::Oidc,
        )
        .expect("refresh token should be created");

        let err = try_build_stateless_oidc_user_session(
            &format!("Bearer {refresh_token}"),
            &ConnectionInfo::new(Some("127.0.0.1".to_string())),
            None,
            &config,
        )
        .expect_err("refresh tokens must be rejected");

        assert!(matches!(err, AuthError::InvalidCredentials(_)));
    }
}

/// Actix-web extractor wrapper for `kalamdb_session::AuthSession`.
///
/// This thin wrapper is required due to Rust's orphan rules (we can't implement
/// `FromRequest` for an external type in this crate). It automatically extracts:
/// - User identity (user_id, role)
/// - Connection info (IP address)
/// - Request tracking ID (X-Request-ID header)
/// - Authentication method (Bearer token)
///
/// **Usage**: Extract this in handlers, then immediately convert to `kalamdb_session::AuthSession`:
///
/// ```rust,ignore
/// use kalamdb_auth::AuthSessionExtractor;
/// use kalamdb_session::AuthSession;
///
/// #[post("/sql")]
/// async fn handler(extractor: AuthSessionExtractor) -> impl Responder {
///     let session: AuthSession = extractor.into();
///     let user_id = session.user_id();
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AuthSessionExtractor(kalamdb_session::AuthSession);

impl From<AuthSessionExtractor> for kalamdb_session::AuthSession {
    fn from(extractor: AuthSessionExtractor) -> Self {
        extractor.0
    }
}

fn try_build_stateless_oidc_user_session(
    auth_header: &str,
    connection_info: &ConnectionInfo,
    request_id: Option<Arc<str>>,
    config: &JwtConfig,
) -> Result<Option<kalamdb_session::AuthSession>, AuthError> {
    let token = extract_bearer_token(auth_header)?;
    let (alg, _header_typ, issuer) = jwt_auth::extract_header_claims_unverified(token)?;

    if !jwt_auth::is_internal_issuer(&issuer) {
        return Ok(None);
    }

    jwt_auth::verify_issuer(&issuer, &config.trusted_issuers)?;

    match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {},
        _ => {
            return Err(AuthError::MalformedAuthorization(
                "Internal 'kalamdb' tokens must use HS256, not an asymmetric algorithm."
                    .to_string(),
            ));
        },
    }

    let claims = jwt_auth::validate_jwt_token(token, &config.secret, &config.trusted_issuers)?;

    match claims.token_type {
        Some(jwt_auth::TokenType::Access) => {},
        Some(jwt_auth::TokenType::Refresh) => {
            return Err(AuthError::InvalidCredentials(
                "Refresh tokens cannot be used for API authentication".to_string(),
            ));
        },
        None => {
            return Err(AuthError::InvalidCredentials(
                "Token missing required token_type claim".to_string(),
            ));
        },
    }

    if !matches!(claims.auth_type, Some(AuthType::Oidc)) || claims.role != Some(Role::User) {
        return Ok(None);
    }

    let user_id = UserId::try_new(claims.sub).map_err(|_| {
        AuthError::MalformedAuthorization("Token contains an invalid user claim".to_string())
    })?;

    let mut session = kalamdb_session::AuthSession::with_auth_details(
        user_id,
        Role::User,
        connection_info.clone(),
        kalamdb_session::AuthMethod::Bearer,
    );

    if let Some(rid) = request_id {
        session = session.with_request_id(rid);
    }

    Ok(Some(session))
}

impl FromRequest for AuthSessionExtractor {
    type Error = AuthExtractError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let req = req.clone();

        Box::pin(async move {
            let start_time = std::time::Instant::now();

            // Get user repository from app data - MUST be registered
            let repo: Arc<dyn UserRepository> = if let Some(repo) =
                req.app_data::<actix_web::web::Data<Arc<dyn UserRepository>>>()
            {
                repo.get_ref().clone()
            } else {
                let took = start_time.elapsed().as_secs_f64() * 1000.0;
                return Err(AuthExtractError::new(
                    AuthError::DatabaseError(
                        "User repository not configured. Ensure Arc<dyn UserRepository> is \
                         registered as app data."
                            .to_string(),
                    ),
                    took,
                ));
            };

            // Extract Authorization header
            let auth_header = match req.headers().get("Authorization") {
                Some(val) => match val.to_str() {
                    Ok(h) => h.to_string(),
                    Err(_) => {
                        let took = start_time.elapsed().as_secs_f64() * 1000.0;
                        return Err(AuthExtractError::new(
                            AuthError::MalformedAuthorization(
                                "Authorization header contains invalid characters".to_string(),
                            ),
                            took,
                        ));
                    },
                },
                None => {
                    let took = start_time.elapsed().as_secs_f64() * 1000.0;
                    return Err(AuthExtractError::new(
                        AuthError::MissingAuthorization(
                            "Authorization header is required. Use 'Authorization: Bearer <token>'"
                                .to_string(),
                        ),
                        took,
                    ));
                },
            };

            // Extract client IP with security checks
            let connection_info = extract_client_ip_secure(&req);

            // Extract X-Request-ID header for audit logging
            let request_id = req
                .headers()
                .get("X-Request-ID")
                .and_then(|h| h.to_str().ok())
                .map(Arc::<str>::from);

            let jwt_config = crate::providers::jwt_config::get_jwt_config();
            match try_build_stateless_oidc_user_session(
                &auth_header,
                &connection_info,
                request_id.clone(),
                &jwt_config,
            ) {
                Ok(Some(session)) => return Ok(AuthSessionExtractor(session)),
                Ok(None) => {},
                Err(e) => {
                    let took = start_time.elapsed().as_secs_f64() * 1000.0;
                    tracing::debug!(
                        error = %e,
                        took_ms = took,
                        "HTTP bearer authentication fast path failed"
                    );
                    return Err(AuthExtractError::new(e, took));
                },
            }

            // Build auth request and authenticate
            let auth_request = AuthRequest::Header(auth_header);

            match authenticate(auth_request, &connection_info, &repo).await {
                Ok(result) => {
                    // Only Bearer tokens are accepted for SQL/API endpoints
                    if !matches!(result.method, kalamdb_session::AuthMethod::Bearer) {
                        let took = start_time.elapsed().as_secs_f64() * 1000.0;
                        return Err(AuthExtractError::new(
                            AuthError::InvalidCredentials(
                                "This endpoint requires a Bearer token.".to_string(),
                            ),
                            took,
                        ));
                    }

                    // Construct AuthSession with all extracted information
                    let mut session = kalamdb_session::AuthSession::with_auth_details(
                        result.user.user_id,
                        result.user.role,
                        connection_info,
                        result.method,
                    );

                    // Add request_id if present
                    if let Some(rid) = request_id {
                        session = session.with_request_id(rid);
                    }

                    Ok(AuthSessionExtractor(session))
                },
                Err(e) => {
                    let took = start_time.elapsed().as_secs_f64() * 1000.0;
                    tracing::debug!(
                        error = %e,
                        took_ms = took,
                        "HTTP bearer authentication failed"
                    );
                    Err(AuthExtractError::new(e, took))
                },
            }
        })
    }
}
