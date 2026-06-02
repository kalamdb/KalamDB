use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{Duration, Utc};
use kalamdb_auth::{
    authenticate, create_and_sign_refresh_token_with_auth_type,
    create_and_sign_token_with_auth_type, create_auth_cookie, create_refresh_cookie,
    extract_client_ip_secure, AuthRequest, CookieConfig, UserRepository,
};
use kalamdb_commons::{AuthType as KalamAuthType, Role};
use kalamdb_configs::{AuthOidcSettings, AuthSettings};
use kalamdb_core::app_context::AppContext;
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    reqwest::Client as OidcHttpClient,
    AuthType, AuthorizationCode, ClientId, ClientSecret, IssuerUrl, PkceCodeVerifier, RedirectUrl,
    TokenResponse as OidcTokenResponse,
};

use super::{
    audit, map_auth_error_to_response,
    models::{
        AuthErrorResponse, LoginResponse, OidcCodeExchangeRequest, OidcTokenExchangeRequest,
        UserInfo,
    },
};
use crate::limiter::RateLimiter;

pub async fn oidc_code_exchange_handler(
    req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
    user_repo: web::Data<Arc<dyn UserRepository>>,
    config: web::Data<AuthSettings>,
    rate_limiter: web::Data<Arc<RateLimiter>>,
    body: web::Json<OidcCodeExchangeRequest>,
) -> HttpResponse {
    let connection_info = extract_client_ip_secure(&req);
    if !rate_limiter.get_ref().check_auth_rate(&connection_info) {
        return HttpResponse::TooManyRequests().json(AuthErrorResponse::new(
            "rate_limited",
            "Too many authentication attempts. Please retry shortly.",
        ));
    }

    let oidc = &app_context.config().auth.oidc;
    if !oidc.enabled {
        return HttpResponse::NotImplemented()
            .json(AuthErrorResponse::new("oidc_unavailable", "OIDC login is not configured"));
    }

    let id_token = match exchange_authorization_code(oidc, &body).await {
        Ok(token) => token,
        Err(message) => {
            log::warn!("OIDC code exchange failed: {}", message);
            return HttpResponse::Unauthorized()
                .json(AuthErrorResponse::new("unauthorized", "Invalid credentials"));
        },
    };

    complete_oidc_login(req, app_context, user_repo, config, id_token, connection_info).await
}

pub async fn oidc_token_exchange_handler(
    req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
    user_repo: web::Data<Arc<dyn UserRepository>>,
    config: web::Data<AuthSettings>,
    rate_limiter: web::Data<Arc<RateLimiter>>,
    body: web::Json<OidcTokenExchangeRequest>,
) -> HttpResponse {
    let connection_info = extract_client_ip_secure(&req);
    if !rate_limiter.get_ref().check_auth_rate(&connection_info) {
        return HttpResponse::TooManyRequests().json(AuthErrorResponse::new(
            "rate_limited",
            "Too many authentication attempts. Please retry shortly.",
        ));
    }

    complete_oidc_login(req, app_context, user_repo, config, body.token.clone(), connection_info)
        .await
}

async fn exchange_authorization_code(
    oidc: &AuthOidcSettings,
    request: &OidcCodeExchangeRequest,
) -> Result<String, String> {
    let issuer = oidc.issuer_str().ok_or_else(|| "auth.oidc.issuer is required".to_string())?;
    let client_id = oidc
        .client_id_str()
        .ok_or_else(|| "auth.oidc.client_id is required".to_string())?;
    let issuer = IssuerUrl::new(issuer.to_string()).map_err(|error| error.to_string())?;
    let http_client = OidcHttpClient::builder()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let metadata = CoreProviderMetadata::discover_async(issuer, &http_client)
        .await
        .map_err(|error| error.to_string())?;
    let client_secret = oidc
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| ClientSecret::new(value.to_string()));
    let redirect_url = RedirectUrl::new(request.redirect_uri.clone())
        .map_err(|error| format!("invalid OIDC redirect_uri in exchange request: {error}"))?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(client_id.to_string()),
        client_secret,
    )
    .set_auth_type(AuthType::RequestBody)
    .set_redirect_uri(redirect_url);

    let token_response = client
        .exchange_code(AuthorizationCode::new(request.code.clone()))
        .map_err(|error| error.to_string())?
        .set_pkce_verifier(PkceCodeVerifier::new(request.code_verifier.clone()))
        .request_async(&http_client)
        .await
        .map_err(|error| error.to_string())?;

    OidcTokenResponse::id_token(&token_response)
        .map(|token| token.to_string())
        .ok_or_else(|| "OIDC provider did not return an ID token".to_string())
}

async fn complete_oidc_login(
    req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
    user_repo: web::Data<Arc<dyn UserRepository>>,
    config: web::Data<AuthSettings>,
    id_token: String,
    connection_info: kalamdb_commons::models::ConnectionInfo,
) -> HttpResponse {
    let auth_result = match authenticate(
        AuthRequest::Jwt { token: id_token },
        &connection_info,
        user_repo.get_ref(),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => return map_auth_error_to_response(error),
    };

    let user = auth_result.user;
    let admin_ui_access = matches!(user.role, Role::Dba | Role::System);
    let (access_token, _) = match create_and_sign_token_with_auth_type(
        &user.user_id,
        &user.role,
        user.email.as_deref(),
        Some(config.jwt_expiry_hours),
        &config.jwt_secret,
        KalamAuthType::Oidc,
    ) {
        Ok(token) => token,
        Err(error) => {
            log::error!("Error generating JWT after OIDC login: {}", error);
            return HttpResponse::InternalServerError()
                .json(AuthErrorResponse::new("internal_error", "Failed to generate token"));
        },
    };

    let refresh_expiry_hours = config.jwt_expiry_hours * 7;
    let (refresh_token, _) = match create_and_sign_refresh_token_with_auth_type(
        &user.user_id,
        &user.role,
        user.email.as_deref(),
        Some(refresh_expiry_hours),
        &config.jwt_secret,
        KalamAuthType::Oidc,
    ) {
        Ok(token) => token,
        Err(error) => {
            log::error!("Error generating refresh token after OIDC login: {}", error);
            return HttpResponse::InternalServerError()
                .json(AuthErrorResponse::new("internal_error", "Failed to generate token"));
        },
    };

    let cookie_config = CookieConfig {
        secure: config.cookie_secure && req.connection_info().scheme() == "https",
        ..Default::default()
    };
    let auth_cookie =
        create_auth_cookie(&access_token, Duration::hours(config.jwt_expiry_hours), &cookie_config);
    let refresh_cookie = create_refresh_cookie(
        &refresh_token,
        Duration::hours(refresh_expiry_hours),
        &cookie_config,
    );

    let expires_at = Utc::now() + Duration::hours(config.jwt_expiry_hours);
    let refresh_expires_at = Utc::now() + Duration::hours(refresh_expiry_hours);
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
            expires_at: expires_at.to_rfc3339(),
            access_token,
            refresh_token,
            refresh_expires_at: refresh_expires_at.to_rfc3339(),
        })
}
