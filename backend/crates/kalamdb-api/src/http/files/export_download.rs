//! Export file download handler.
//!
//! Serves completed user data export ZIP archives.
//!
//! ## Endpoint
//! GET /v1/exports/{user_id}/{export_id}
//!
//! Downloading transfer artifacts requires a DBA or System role.

use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::models::UserId;
use kalamdb_core::app_context::AppContext;
use kalamdb_session::{is_admin_role, AuthSession};
use kalamdb_transfer::{is_safe_transfer_id, table_export_zip_path, user_export_zip_path};

use crate::http::sql::models::{ErrorCode, SqlResponse};

/// GET /v1/exports/{user_id}/{export_id} - Download a user data export ZIP
///
/// Requires Bearer token (JWT) authorization.
/// Only DBA/System roles can download exports.
#[get("/exports/{user_id}/{export_id}")]
pub async fn download_export(
    extractor: AuthSessionExtractor,
    path: web::Path<(String, String)>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    let (user_id_raw, export_id) = path.into_inner();

    let user_id = match UserId::try_new(user_id_raw) {
        Ok(user_id) => user_id,
        Err(_) => {
            return HttpResponse::BadRequest().json(SqlResponse::error(
                ErrorCode::InvalidInput,
                "Invalid export path",
                0.0,
            ));
        },
    };

    if !is_safe_transfer_id(&export_id) {
        return HttpResponse::BadRequest().json(SqlResponse::error(
            ErrorCode::InvalidInput,
            "Invalid export path",
            0.0,
        ));
    }

    if session.user_id() != &user_id && !is_admin_role(session.role()) {
        return HttpResponse::Forbidden().json(SqlResponse::error(
            ErrorCode::PermissionDenied,
            "Only the export owner, DBA, or System role may download exports",
            0.0,
        ));
    }

    // Build file path
    let exports_dir = app_context.config().storage.exports_dir();
    let zip_path = user_export_zip_path(&exports_dir, user_id.as_str(), &export_id);

    serve_export_zip(&zip_path, &export_id, "Export").await
}

/// GET /v1/table-exports/{export_id} - Download a single-table export ZIP.
///
/// Requires DBA/System role because table exports may contain shared data or
/// administrator-selected user data.
#[get("/table-exports/{export_id}")]
pub async fn download_table_export(
    extractor: AuthSessionExtractor,
    path: web::Path<String>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    let export_id = path.into_inner();

    if !is_safe_transfer_id(&export_id) {
        return HttpResponse::BadRequest().json(SqlResponse::error(
            ErrorCode::InvalidInput,
            "Invalid export path",
            0.0,
        ));
    }

    if !is_admin_role(session.role()) {
        return HttpResponse::Forbidden().json(SqlResponse::error(
            ErrorCode::PermissionDenied,
            "DBA or System role is required to download table exports",
            0.0,
        ));
    }

    let exports_dir = app_context.config().storage.exports_dir();
    let zip_path = table_export_zip_path(&exports_dir, &export_id);

    serve_export_zip(&zip_path, &export_id, "Table export").await
}

async fn serve_export_zip(
    zip_path: &std::path::Path,
    export_id: &str,
    log_prefix: &str,
) -> HttpResponse {
    if !zip_path.exists() {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Export not found",
            "code": "EXPORT_NOT_FOUND",
        }));
    }

    match tokio::fs::read(zip_path).await {
        Ok(data) => {
            let filename = format!("{}.zip", export_id);
            HttpResponse::Ok()
                .content_type("application/zip")
                .append_header((
                    "Content-Disposition",
                    format!("attachment; filename=\"{}\"", filename),
                ))
                .body(data)
        },
        Err(error) => {
            log::warn!(
                "{} download failed: path={}, error={}",
                log_prefix,
                zip_path.display(),
                error
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to read export file",
                "code": "INTERNAL_ERROR",
            }))
        },
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_transfer::is_safe_transfer_id;

    #[test]
    fn export_id_accepts_generated_shape() {
        assert!(is_safe_transfer_id("export-alice_1-20260101-120000"));
    }

    #[test]
    fn export_id_rejects_path_and_header_injection() {
        for value in [
            "",
            "../escape",
            "export/alice",
            "export\\alice",
            "export\nalice",
            "export\ralice",
            "export;DROP",
            "export'quote",
            "export.zip",
            "éxport",
        ] {
            assert!(!is_safe_transfer_id(value), "expected rejection for {value:?}");
        }
    }
}
