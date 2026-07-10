// KalamDB Authentication Library
// Provides password hashing, JWT validation, Basic Auth, and authorization
//
// SINGLE SOURCE OF TRUTH: All authentication logic goes through the `unified` module.
// Both HTTP (/sql endpoint) and WebSocket handlers use the same authentication flow.

mod oidc;

pub mod authorization;
pub mod errors;
pub mod helpers;
pub mod models;
pub mod providers;
pub mod repository;
pub mod security;
pub mod services;

// Re-export commonly used types
pub use errors::error::{AuthError, AuthResult};
pub use helpers::authorization_header::{
    extract_bearer_token, is_basic_authorization, parse_authorization_header, AuthorizationScheme,
    ParsedAuthorizationHeader,
};
#[cfg(feature = "http")]
pub use helpers::cookie::{
    create_auth_cookie, create_logout_cookie, create_refresh_cookie, create_refresh_logout_cookie,
    extract_auth_token, extract_refresh_token, CookieConfig, AUTH_COOKIE_NAME,
};
#[cfg(feature = "http")]
pub use helpers::extractor::{AuthExtractError, AuthSessionExtractor};
// Re-export items needed by extractor
#[cfg(feature = "http")]
pub use helpers::ip_extractor::{
    extract_client_ip_addr_secure, extract_client_ip_secure, init_trusted_proxy_ranges,
    is_localhost_address,
};
#[cfg(feature = "http")]
pub use helpers::request_tokens::{
    extract_bearer_or_cookie_token, extract_refresh_or_bearer_token,
};
pub use models::impersonation::{ImpersonationContext, ImpersonationOrigin};
pub use models::login_request::LoginRequest;
pub use providers::jwt_auth::{
    create_and_sign_refresh_token, create_and_sign_refresh_token_with_auth_type,
    create_and_sign_token, create_and_sign_token_with_auth_type, generate_jwt_token,
    refresh_jwt_token, JwtClaims, TokenType, DEFAULT_JWT_EXPIRY_HOURS, KALAMDB_ISSUER,
};
pub use repository::user_repo::{CachedUsersRepo, CoreUsersRepo, UserRepository};
#[cfg(feature = "http")]
pub use services::session_tokens::auth_cookie_config;
pub use services::{
    login_tracker::{LoginTracker, LoginTrackingConfig},
    session_tokens::{issue_auth_tokens, IssuedAuthTokens},
    unified::{
        authenticate, authenticate_wire_password, extract_user_id_for_audit,
        resolve_refresh_token_user, AuthRequest, AuthenticationResult, WireAuthResult,
        WirePasswordAuthRequest,
    },
};
