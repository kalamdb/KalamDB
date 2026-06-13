//! Logout handler
//!
//! POST /v1/api/auth/logout - Clears authentication cookie

use actix_web::{web, HttpRequest, HttpResponse};
use kalamdb_auth::{auth_cookie_config, create_logout_cookie, create_refresh_logout_cookie};
use kalamdb_configs::AuthSettings;

/// POST /v1/api/auth/logout
///
/// Clears the authentication cookie.
pub async fn logout_handler(_req: HttpRequest, config: web::Data<AuthSettings>) -> HttpResponse {
    let cookie_config = auth_cookie_config(config.get_ref());
    let auth_cookie = create_logout_cookie(&cookie_config);
    let refresh_cookie = create_refresh_logout_cookie(&cookie_config);

    HttpResponse::Ok()
        .cookie(auth_cookie)
        .cookie(refresh_cookie)
        .json(serde_json::json!({
            "message": "Logged out successfully"
        }))
}
