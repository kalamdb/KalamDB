use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use kalamdb_auth::{
    auth_cookie_config, authenticate, create_auth_cookie, create_refresh_cookie,
    extract_client_ip_secure, issue_auth_tokens,
    services::oidc_device::{poll_oidc_device_flow, start_oidc_device_flow, OidcDevicePollResult},
    AuthRequest, UserRepository,
};
use kalamdb_commons::{AuthType, Role};
use kalamdb_configs::AuthSettings;
use kalamdb_core::app_context::AppContext;

use super::{
    audit, map_auth_error_to_response,
    models::{
        AuthErrorResponse, OidcDevicePollRequest, OidcDevicePollResponse, OidcDevicePollStatus,
        OidcDeviceStartRequest, OidcDeviceStartResponse, UserInfo,
    },
};
use crate::limiter::RateLimiter;

pub async fn oidc_device_start_handler(
    req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
    config: web::Data<AuthSettings>,
    rate_limiter: web::Data<Arc<RateLimiter>>,
    request: web::Json<OidcDeviceStartRequest>,
) -> HttpResponse {
    let connection_info = extract_client_ip_secure(&req);
    if !rate_limiter.get_ref().check_auth_rate(&connection_info) {
        return HttpResponse::TooManyRequests().json(AuthErrorResponse::new(
            "rate_limited",
            "Too many authentication attempts. Please retry shortly.",
        ));
    }

    let oidc = &app_context.config().auth.oidc;
    if !oidc.enabled || !oidc.broker_device_flow_enabled {
        return HttpResponse::NotImplemented().json(AuthErrorResponse::new(
            "oidc_device_flow_unavailable",
            "OIDC device flow is not configured",
        ));
    }

    match start_oidc_device_flow(oidc, &request.scopes).await {
        Ok(result) => HttpResponse::Ok().json(OidcDeviceStartResponse {
            device_session_id: result.device_session_id,
            verification_uri: result.verification_uri,
            verification_uri_complete: result.verification_uri_complete,
            user_code: result.user_code,
            expires_in_seconds: result.expires_in_seconds,
            interval_seconds: result.interval_seconds,
        }),
        Err(error) => {
            log::warn!("OIDC device start failed: {}", error);
            let _ = config;
            HttpResponse::ServiceUnavailable().json(AuthErrorResponse::new(
                "oidc_device_flow_unavailable",
                "OIDC device flow is not available",
            ))
        },
    }
}

pub async fn oidc_device_poll_handler(
    req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
    user_repo: web::Data<Arc<dyn UserRepository>>,
    config: web::Data<AuthSettings>,
    rate_limiter: web::Data<Arc<RateLimiter>>,
    request: web::Json<OidcDevicePollRequest>,
) -> HttpResponse {
    let connection_info = extract_client_ip_secure(&req);
    if !rate_limiter.get_ref().check_auth_rate(&connection_info) {
        return HttpResponse::TooManyRequests().json(AuthErrorResponse::new(
            "rate_limited",
            "Too many authentication attempts. Please retry shortly.",
        ));
    }

    let poll_result = match poll_oidc_device_flow(&request.device_session_id).await {
        Ok(result) => result,
        Err(error) => return map_auth_error_to_response(error),
    };

    match poll_result {
        OidcDevicePollResult::Pending { interval_seconds } => {
            HttpResponse::Ok().json(OidcDevicePollResponse {
                status: OidcDevicePollStatus::Pending,
                interval_seconds: Some(interval_seconds),
                token_type: None,
                access_token: None,
                expires_at: None,
                refresh_token: None,
                refresh_expires_at: None,
                user: None,
                admin_ui_access: None,
                message: None,
            })
        },
        OidcDevicePollResult::Authorized { id_token } => {
            complete_authorized_device_login(
                app_context,
                user_repo,
                config,
                id_token,
                connection_info,
            )
            .await
        },
        OidcDevicePollResult::Denied { message } => {
            terminal_poll_response(OidcDevicePollStatus::Denied, message)
        },
        OidcDevicePollResult::Expired { message } => {
            terminal_poll_response(OidcDevicePollStatus::Expired, message)
        },
        OidcDevicePollResult::Failed { message } => {
            terminal_poll_response(OidcDevicePollStatus::Failed, message)
        },
    }
}

async fn complete_authorized_device_login(
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
    let issued_tokens = match issue_auth_tokens(
        &user.user_id,
        &user.role,
        user.email.as_deref(),
        AuthType::Oidc,
        config.get_ref(),
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            log::error!("Error generating tokens after OIDC device login: {}", error);
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
        .json(OidcDevicePollResponse {
            status: OidcDevicePollStatus::Authorized,
            interval_seconds: None,
            token_type: Some("bearer".to_string()),
            access_token: Some(issued_tokens.access_token),
            expires_at: Some(issued_tokens.expires_at.to_rfc3339()),
            refresh_token: Some(issued_tokens.refresh_token),
            refresh_expires_at: Some(issued_tokens.refresh_expires_at.to_rfc3339()),
            user: Some(UserInfo {
                id: user.user_id,
                role: user.role,
                name: user.name,
                email: user.email,
                created_at,
                updated_at,
            }),
            admin_ui_access: Some(admin_ui_access),
            message: None,
        })
}

fn terminal_poll_response(status: OidcDevicePollStatus, message: String) -> HttpResponse {
    HttpResponse::Ok().json(OidcDevicePollResponse {
        status,
        interval_seconds: None,
        token_type: None,
        access_token: None,
        expires_at: None,
        refresh_token: None,
        refresh_expires_at: None,
        user: None,
        admin_ui_access: None,
        message: Some(message),
    })
}
