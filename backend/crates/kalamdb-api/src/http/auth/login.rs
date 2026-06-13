//! Login handler
//!
//! POST /v1/api/auth/login - Authenticates a user and returns JWT tokens

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use kalamdb_auth::{
    auth_cookie_config, authenticate, create_auth_cookie, create_refresh_cookie,
    extract_client_ip_secure, issue_auth_tokens, AuthRequest, UserRepository,
};
use kalamdb_commons::{AuthType, Role};
use kalamdb_configs::AuthSettings;
use kalamdb_core::app_context::AppContext;
use kalamdb_jobs::health_monitor::record_activity_now;

use super::{
    audit, map_auth_error_to_response,
    models::{AuthErrorResponse, LoginRequest, LoginResponse, UserInfo},
};
use crate::limiter::RateLimiter;

/// POST /v1/api/auth/login
///
/// Authenticates a user and returns JWT tokens for API usage.
/// The response also includes `admin_ui_access` so browser clients can
/// distinguish normal API tokens from accounts allowed to enter the Admin UI.
pub async fn login_handler(
    req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
    user_repo: web::Data<Arc<dyn UserRepository>>,
    config: web::Data<AuthSettings>,
    rate_limiter: web::Data<Arc<RateLimiter>>,
    body: web::Json<LoginRequest>,
) -> HttpResponse {
    record_activity_now();

    // Extract client IP with anti-spoofing checks for localhost validation
    let connection_info = extract_client_ip_secure(&req);

    // Rate limit auth attempts by client IP
    if !rate_limiter.get_ref().check_auth_rate(&connection_info) {
        return HttpResponse::TooManyRequests().json(AuthErrorResponse::new(
            "rate_limited",
            "Too many authentication attempts. Please retry shortly.",
        ));
    }

    if !config.local.enabled {
        return HttpResponse::Forbidden().json(AuthErrorResponse::new(
            "local_auth_disabled",
            "Local username/password login is disabled. Use the configured OIDC login method.",
        ));
    }

    // Authenticate using unified auth flow (includes localhost/empty password rules)
    let auth_request = AuthRequest::Credentials {
        user: body.user.clone(),
        password: body.password.clone(),
    };

    let auth_result = match authenticate(auth_request, &connection_info, user_repo.get_ref()).await
    {
        Ok(result) => result,
        Err(err) => return map_auth_error_to_response(err),
    };

    let user = auth_result.user;
    let admin_ui_access = matches!(user.role, Role::Dba | Role::System);

    let issued_tokens = match issue_auth_tokens(
        &user.user_id,
        &user.role,
        user.email.as_deref(),
        AuthType::Password,
        config.get_ref(),
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            log::error!("Error generating login tokens: {}", error);
            return HttpResponse::InternalServerError()
                .json(AuthErrorResponse::new("internal_error", "Failed to generate token"));
        },
    };

    let cookie_config = auth_cookie_config(config.get_ref());
    let auth_cookie =
        create_auth_cookie(&issued_tokens.access_token, issued_tokens.access_expires_in, &cookie_config);
    let refresh_cookie = create_refresh_cookie(
        &issued_tokens.refresh_token,
        issued_tokens.refresh_expires_in,
        &cookie_config,
    );

    // Convert timestamps properly
    let created_at = chrono::DateTime::from_timestamp_millis(user.created_at)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    let updated_at = chrono::DateTime::from_timestamp_millis(user.updated_at)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    if admin_ui_access {
        audit::record_admin_login(app_context.get_ref(), &user.user_id, &connection_info).await;
    }

    HttpResponse::Ok()
        .cookie(auth_cookie)
        .cookie(refresh_cookie)
        .json(LoginResponse {
            user: UserInfo {
                id: user.user_id,
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
