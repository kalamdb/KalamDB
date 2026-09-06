//! File download handler

use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::{
    models::{TableId, UserId},
    schemas::TableType,
    Role,
};
use kalamdb_core::app_context::AppContext;
use kalamdb_session::{can_access_user_table, can_impersonate_target_user, AuthSession};
use kalamdb_system::FileRef;

use super::models::DownloadQuery;
use crate::http::sql::models::{ErrorCode, SqlResponse};

/// GET /v1/files/{namespace}/{table_name}/{subfolder}/{stored_name} - Download a file
///
/// Requires Bearer token (JWT) authorization and table access permissions.
/// For user tables, downloads default to the authenticated user's table scope.
/// DBA/system roles may supply `user_id` when the impersonation role matrix allows it.
#[get("/files/{namespace}/{table_name}/{subfolder}/{stored_name}")]
pub async fn download_file(
    extractor: AuthSessionExtractor,
    path: web::Path<(String, String, String, String)>,
    query: web::Query<DownloadQuery>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    // Convert extractor to AuthSession
    let session: AuthSession = extractor.into();

    let (namespace, table_name, subfolder, stored_name) = path.into_inner();
    let table_id = TableId::from_strings(&namespace, &table_name);

    // Look up table definition from schema registry
    let schema_registry = app_context.schema_registry();
    let table_entry = match schema_registry.get(&table_id) {
        Some(cached) => cached.table_entry(),
        None => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "error": format!("Table '{}' not found", table_id),
            }));
        },
    };

    let storage_id = table_entry.storage_id.clone();
    let table_type = table_entry.table_type;

    let user_id = match table_type {
        TableType::User => {
            if !can_access_user_table(session.role()) {
                return HttpResponse::Forbidden().json(SqlResponse::error(
                    ErrorCode::PermissionDenied,
                    "User table file downloads require user-table access",
                    0.0,
                ));
            }

            let effective_user_id = if let Some(requested_user_id) = query.user_id.as_ref() {
                let target_role = app_context
                    .system_tables()
                    .users()
                    .role_for_impersonation_target(requested_user_id);

                if !can_download_user_file_for_target(&session, requested_user_id, target_role) {
                    return HttpResponse::Forbidden().json(SqlResponse::error(
                        ErrorCode::PermissionDenied,
                        "Requested user is not allowed for file download",
                        0.0,
                    ));
                }

                requested_user_id.clone()
            } else {
                session.user_id().clone()
            };

            Some(effective_user_id)
        },
        TableType::Shared => {
            if !can_download_shared_file(session.role()) {
                return HttpResponse::Forbidden().json(SqlResponse::error(
                    ErrorCode::PermissionDenied,
                    "Raw shared-table file downloads require DBA or System role",
                    0.0,
                ));
            }
            if query.user_id.is_some() {
                return HttpResponse::BadRequest().json(SqlResponse::error(
                    ErrorCode::InvalidInput,
                    "user_id is only valid for user tables",
                    0.0,
                ));
            }
            None
        },
        TableType::Stream | TableType::System => {
            // Stream and system tables don't support file storage
            return HttpResponse::BadRequest().json(SqlResponse::error(
                ErrorCode::InvalidInput,
                "File storage is not supported for stream or system tables",
                0.0,
            ));
        },
    };

    // Validate path components for security
    let subfolder_is_valid = FileRef::is_valid_subfolder(&subfolder);

    if !subfolder_is_valid
        || subfolder.contains("..")
        || subfolder.contains('/')
        || subfolder.contains('\\')
        || subfolder.contains('\0')
        || stored_name.contains("..")
        || stored_name.contains('/')
        || stored_name.contains('\\')
        || stored_name.contains('\0')
    {
        return HttpResponse::BadRequest().json(SqlResponse::error(
            ErrorCode::InvalidInput,
            "Invalid file path",
            0.0,
        ));
    }
    let relative_path = format!("{}/{}", subfolder, stored_name);

    // Fetch file from storage
    let file_service = app_context.file_storage_service();
    match file_service
        .get_file_by_path(&storage_id, table_type, &table_id, user_id.as_ref(), &relative_path)
        .await
    {
        Ok(data) => {
            // TODO: Get content type from the stored file metadata
            // Guess content type from file extension in stored_name
            let content_type = guess_content_type(&stored_name);

            // SECURITY: Sanitize stored_name for Content-Disposition header to prevent
            // HTTP response header injection (CRLF injection) via crafted filenames.
            let safe_stored_name: String = stored_name
                .chars()
                .filter(|c| *c != '"' && *c != '\r' && *c != '\n' && *c != '\0')
                .collect();
            HttpResponse::Ok()
                .content_type(content_type)
                .append_header((
                    "Content-Disposition",
                    format!("inline; filename=\"{}\"", safe_stored_name),
                ))
                .body(data)
        },
        Err(e) => {
            log::warn!("File download failed: table={}, file={}: {}", table_id, stored_name, e);
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "File not found",
                "code": "FILE_NOT_FOUND",
            }))
        },
    }
}

fn can_download_shared_file(role: Role) -> bool {
    matches!(role, Role::Dba | Role::System)
}

fn guess_content_type(stored_name: &str) -> String {
    mime_guess::from_path(stored_name).first_or_octet_stream().to_string()
}

fn can_download_user_file_for_target(
    session: &AuthSession,
    requested_user_id: &UserId,
    target_role: Role,
) -> bool {
    if requested_user_id == session.user_id() {
        return true;
    }

    matches!(session.role(), Role::System | Role::Dba)
        && can_impersonate_target_user(
            session.user_id(),
            session.role(),
            requested_user_id,
            target_role,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_user_id_query_is_self_or_admin_only() {
        let service = AuthSession::new(UserId::new("svc"), Role::Service);
        let dba = AuthSession::new(UserId::new("dba"), Role::Dba);
        let system = AuthSession::new(UserId::new("system"), Role::System);
        let user = AuthSession::new(UserId::new("alice"), Role::User);
        let regular_target = UserId::new("alice");
        let dba_target = UserId::new("dba-target");

        assert!(can_download_user_file_for_target(&user, &regular_target, Role::User));
        assert!(!can_download_user_file_for_target(&service, &regular_target, Role::User));
        assert!(can_download_user_file_for_target(&dba, &regular_target, Role::User));
        assert!(can_download_user_file_for_target(&dba, &dba_target, Role::Dba));
        assert!(can_download_user_file_for_target(&system, &dba_target, Role::Dba));
    }

    #[test]
    fn shared_file_downloads_are_admin_only_even_for_services() {
        assert!(!can_download_shared_file(Role::User));
        assert!(!can_download_shared_file(Role::Service));
        assert!(can_download_shared_file(Role::Dba));
        assert!(can_download_shared_file(Role::System));
    }
}
