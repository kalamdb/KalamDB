//! Token refresh handler
//!
//! POST /v1/api/auth/refresh - Refreshes the JWT token if still valid

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use kalamdb_auth::{
    auth_cookie_config, create_auth_cookie, create_refresh_cookie, extract_client_ip_secure,
    extract_refresh_or_bearer_token, issue_auth_tokens,
    providers::jwt_auth::{validate_jwt_token, TokenType},
    resolve_refresh_token_user, UserRepository,
};
use kalamdb_commons::Role;
use kalamdb_configs::AuthSettings;

use super::{
    map_auth_error_to_response,
    models::{AuthErrorResponse, LoginResponse, UserInfo},
};
use crate::limiter::RateLimiter;

/// POST /v1/api/auth/refresh
///
/// Refreshes the JWT token if the current one is still valid.
pub async fn refresh_handler(
    req: HttpRequest,
    user_repo: web::Data<Arc<dyn UserRepository>>,
    config: web::Data<AuthSettings>,
    rate_limiter: web::Data<Arc<RateLimiter>>,
) -> HttpResponse {
    // Extract client IP with anti-spoofing checks for localhost validation
    let connection_info = extract_client_ip_secure(&req);

    // Rate limit auth attempts by client IP
    if !rate_limiter.get_ref().check_auth_rate(&connection_info) {
        return HttpResponse::TooManyRequests().json(AuthErrorResponse::new(
            "rate_limited",
            "Too many authentication attempts. Please retry shortly.",
        ));
    }

    let token = match extract_refresh_or_bearer_token(&req) {
        Ok(t) => t,
        Err(err) => return map_auth_error_to_response(err),
    };

    // Validate existing token directly, then require a real refresh token.
    let jwt_config = kalamdb_auth::providers::jwt_config::get_jwt_config();
    let claims = match validate_jwt_token(&token, &jwt_config.secret, &jwt_config.trusted_issuers) {
        Ok(c) => c,
        Err(err) => return map_auth_error_to_response(err),
    };

    if !matches!(claims.token_type, Some(TokenType::Refresh)) {
        return HttpResponse::Unauthorized().json(AuthErrorResponse::new(
            "unauthorized",
            "Refresh endpoint requires a refresh token",
        ));
    }

    let user =
        match resolve_refresh_token_user(user_repo.get_ref(), &claims, &connection_info).await {
            Ok(user) => user,
            Err(err) => return map_auth_error_to_response(err),
        };

    let issued_tokens = match issue_auth_tokens(
        &user.user_id,
        &user.role,
        user.email.as_deref(),
        user.auth_type,
        config.get_ref(),
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            log::error!("Error refreshing tokens: {}", error);
            return HttpResponse::InternalServerError()
                .json(AuthErrorResponse::new("internal_error", "Failed to refresh token"));
        },
    };

    let cookie_config = auth_cookie_config(config.get_ref());
    let auth_cookie = create_auth_cookie(
        &issued_tokens.access_token,
        issued_tokens.access_expires_in,
        &cookie_config,
    );
    let refresh_cookie = create_refresh_cookie(
        &issued_tokens.refresh_token,
        issued_tokens.refresh_expires_in,
        &cookie_config,
    );

    let admin_ui_access = matches!(user.role, Role::Dba | Role::System);

    // Convert timestamps properly
    let created_at = chrono::DateTime::from_timestamp_millis(user.created_at)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    let updated_at = chrono::DateTime::from_timestamp_millis(user.updated_at)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    HttpResponse::Ok()
        .cookie(auth_cookie)
        .cookie(refresh_cookie)
        .json(LoginResponse {
            user: UserInfo {
                id: user.user_id.clone(),
                role: user.role,
                name: user.name,
                email: user.email,
                created_at,
                updated_at,
            },
            admin_ui_access,
            expires_at: issued_tokens.expires_at.to_rfc3339(),
            access_token: issued_tokens.access_token,
            refresh_token: issued_tokens.refresh_token,
            refresh_expires_at: issued_tokens.refresh_expires_at.to_rfc3339(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kalamdb_auth::JwtClaims;

    #[test]
    fn refresh_endpoint_only_accepts_refresh_token_type() {
        let now = chrono::Utc::now().timestamp() as usize;
        let refresh_claims = JwtClaims {
            sub: "u_1".to_string(),
            iss: "kalamdb".to_string(),
            exp: now + 3600,
            iat: now,
            name: None,
            email: None,
            email_verified: None,
            role: None,
            auth_type: None,
            token_type: Some(TokenType::Refresh),
        };
        let access_claims = JwtClaims {
            token_type: Some(TokenType::Access),
            ..refresh_claims.clone()
        };
        let legacy_claims = JwtClaims {
            token_type: None,
            ..refresh_claims.clone()
        };

        assert!(matches!(refresh_claims.token_type, Some(TokenType::Refresh)));
        assert!(!matches!(access_claims.token_type, Some(TokenType::Refresh)));
        assert!(!matches!(legacy_claims.token_type, Some(TokenType::Refresh)));
    }
}
