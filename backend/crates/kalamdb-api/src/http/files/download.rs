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
/// User-table downloads stay in the caller's user id. DBA and System may supply
/// `user_id` when the impersonation role matrix allows it.
/// Shared-table downloads follow SELECT visibility: DBA and System read the blob
/// directly, and User or Service read it only when a current row they can select
/// stores this FileRef. A hidden or missing file uses the same not-found body.
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

    // Traversal stays a bad request. A subfolder that is not a minted FileRef
    // folder is indistinguishable from a missing file.
    if path_has_traversal(&subfolder) || path_has_traversal(&stored_name) {
        return HttpResponse::BadRequest().json(SqlResponse::error(
            ErrorCode::InvalidInput,
            "Invalid file path",
            0.0,
        ));
    }
    if !FileRef::is_valid_subfolder(&subfolder) {
        return file_not_found();
    }
    let relative_path = format!("{}/{}", subfolder, stored_name);

    if table_type == TableType::Shared && !shared_file_download_bypasses_rls(session.role()) {
        match app_context
            .shared_select_authorizes_file(
                &table_id,
                session.user_id(),
                session.role(),
                &subfolder,
                &stored_name,
            )
            .await
        {
            Ok(true) => {},
            Ok(false) => return file_not_found(),
            Err(error) => {
                log::warn!(
                    "Shared file visibility check failed: table={}, file={}: {}",
                    table_id,
                    stored_name,
                    error
                );
                return HttpResponse::InternalServerError().json(SqlResponse::error(
                    ErrorCode::InternalError,
                    "Failed to authorize file download",
                    0.0,
                ));
            },
        }
    }

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
            file_not_found()
        },
    }
}

fn path_has_traversal(component: &str) -> bool {
    component.contains("..")
        || component.contains('/')
        || component.contains('\\')
        || component.contains('\0')
}

fn file_not_found() -> HttpResponse {
    HttpResponse::NotFound().json(serde_json::json!({
        "error": "File not found",
        "code": "FILE_NOT_FOUND",
    }))
}

fn shared_file_download_bypasses_rls(role: Role) -> bool {
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
    fn shared_file_download_bypass_is_dba_or_system() {
        assert!(!shared_file_download_bypasses_rls(Role::User));
        assert!(!shared_file_download_bypasses_rls(Role::Service));
        assert!(shared_file_download_bypasses_rls(Role::Dba));
        assert!(shared_file_download_bypasses_rls(Role::System));
    }
}
