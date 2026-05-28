//! Admin table data transfer endpoints.
//!
//! These endpoints back the SQL Studio table editor export/import controls.

use std::{fs, sync::Arc};

use actix_multipart::Multipart;
use actix_web::{get, post, web, HttpResponse, Responder};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::{JobId, NamespaceId, TableId, TableName};
use kalamdb_core::app_context::AppContext;
use kalamdb_jobs::{
    executors::table_transfer::{TableExportParams, TableImportParams},
    AppContextJobsExt,
};
use kalamdb_session::{is_admin_role, AuthSession};
use kalamdb_system::{providers::jobs::models::Job, JobStatus, JobType};
use kalamdb_transfer::{
    build_table_export_download_url, generate_table_export_id, generate_table_import_id,
    table_import_zip_path, validate_table_transfer_scope,
};
use serde::{Deserialize, Serialize};

const MAX_IMPORT_ZIP_BYTES: usize = 100 * 1024 * 1024;
const MAX_TEXT_FIELD_BYTES: usize = 4096;

#[derive(Debug, Deserialize)]
pub struct TableExportRequest {
    pub namespace_id: String,
    pub table_name: String,
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TableTransferJobResponse {
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
}

#[derive(Default)]
struct ParsedImportMultipart {
    namespace_id: Option<String>,
    table_name: Option<String>,
    user_id: Option<String>,
    file: Option<Bytes>,
}

/// POST /v1/api/table-exports - Start a single-table data export job.
#[post("/table-exports")]
pub async fn start_table_export(
    extractor: AuthSessionExtractor,
    body: web::Json<TableExportRequest>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    if !is_admin_role(session.role()) {
        return forbidden("DBA or System role is required to export table data");
    }

    let (table_id, table_type, user_id) = match parse_transfer_request(&body, app_context.as_ref()) {
        Ok(value) => value,
        Err(message) => return bad_request(message),
    };

    let export_id = generate_table_export_id();
    let params = TableExportParams {
        table_id: table_id.clone(),
        table_type,
        user_id,
        export_id: export_id.clone(),
    };

    let job_manager = app_context.job_manager();
    match job_manager.create_job_typed(JobType::TableExport, params, None, None).await {
        Ok(job_id) => HttpResponse::Ok().json(TableTransferJobResponse {
            job_id: job_id.as_str().to_string(),
            export_id: Some(export_id.clone()),
            import_id: None,
            status: JobStatus::Queued.as_str().to_string(),
            message: Some(format!("Table export queued for {}", table_id.full_name())),
            download_url: Some(build_table_export_download_url(&export_id)),
        }),
        Err(error) => internal_error(format!("Failed to start table export: {}", error)),
    }
}

/// GET /v1/api/table-exports/{job_id} - Poll table export job status.
#[get("/table-exports/{job_id}")]
pub async fn get_table_export_status(
    extractor: AuthSessionExtractor,
    path: web::Path<String>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    get_transfer_status(extractor, path, app_context, JobType::TableExport).await
}

/// POST /v1/api/table-imports - Upload a table export ZIP and start an import job.
#[post("/table-imports")]
pub async fn start_table_import(
    extractor: AuthSessionExtractor,
    payload: Multipart,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    if !is_admin_role(session.role()) {
        return forbidden("DBA or System role is required to import table data");
    }

    let parsed = match parse_import_multipart(payload).await {
        Ok(value) => value,
        Err(message) => return bad_request(message),
    };

    let request = TableExportRequest {
        namespace_id: parsed.namespace_id.unwrap_or_default(),
        table_name: parsed.table_name.unwrap_or_default(),
        user_id: parsed.user_id,
    };
    let (table_id, table_type, user_id) = match parse_transfer_request(&request, app_context.as_ref()) {
        Ok(value) => value,
        Err(message) => return bad_request(message),
    };
    let Some(file) = parsed.file else {
        return bad_request("Import ZIP file is required".to_string());
    };

    let import_id = generate_table_import_id();
    let imports_dir = app_context.config().storage.exports_dir().join("imports");
    if let Err(error) = fs::create_dir_all(&imports_dir) {
        return internal_error(format!(
            "Failed to create imports directory '{}': {}",
            imports_dir.display(),
            error
        ));
    }
    let zip_path = table_import_zip_path(&app_context.config().storage.exports_dir(), &import_id);
    if let Err(error) = fs::write(&zip_path, &file) {
        return internal_error(format!(
            "Failed to stage import ZIP '{}': {}",
            zip_path.display(),
            error
        ));
    }

    let params = TableImportParams {
        table_id: table_id.clone(),
        table_type,
        user_id,
        import_id: import_id.clone(),
    };

    let job_manager = app_context.job_manager();
    match job_manager.create_job_typed(JobType::TableImport, params, None, None).await {
        Ok(job_id) => HttpResponse::Ok().json(TableTransferJobResponse {
            job_id: job_id.as_str().to_string(),
            export_id: None,
            import_id: Some(import_id),
            status: JobStatus::Queued.as_str().to_string(),
            message: Some(format!("Table import queued for {}", table_id.full_name())),
            download_url: None,
        }),
        Err(error) => {
            let _ = fs::remove_file(&zip_path);
            internal_error(format!("Failed to start table import: {}", error))
        },
    }
}

/// GET /v1/api/table-imports/{job_id} - Poll table import job status.
#[get("/table-imports/{job_id}")]
pub async fn get_table_import_status(
    extractor: AuthSessionExtractor,
    path: web::Path<String>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    get_transfer_status(extractor, path, app_context, JobType::TableImport).await
}

async fn get_transfer_status(
    extractor: AuthSessionExtractor,
    path: web::Path<String>,
    app_context: web::Data<Arc<AppContext>>,
    expected_type: JobType,
) -> HttpResponse {
    let session: AuthSession = extractor.into();
    if !is_admin_role(session.role()) {
        return forbidden("DBA or System role is required to view table transfer jobs");
    }

    let job_id = JobId::new(path.into_inner());
    match app_context.job_manager().get_job(&job_id).await {
        Ok(Some(job)) if job.job_type == expected_type => {
            HttpResponse::Ok().json(job_response(&job))
        },
        Ok(Some(_)) => bad_request("Job type does not match this endpoint".to_string()),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "not_found",
            "message": "Transfer job not found",
        })),
        Err(error) => internal_error(format!("Failed to read transfer job: {}", error)),
    }
}

fn job_response(job: &Job) -> TableTransferJobResponse {
    let export_id = job_parameter(job, "export_id");
    let import_id = job_parameter(job, "import_id");
    let download_url = if job.job_type == JobType::TableExport && job.status == JobStatus::Completed
    {
        export_id.as_deref().map(build_table_export_download_url)
    } else {
        None
    };

    TableTransferJobResponse {
        job_id: job.job_id.as_str().to_string(),
        export_id,
        import_id,
        status: job.status.as_str().to_string(),
        message: job.message.clone(),
        download_url,
    }
}

fn job_parameter(job: &Job, key: &str) -> Option<String> {
    let parameters = job.parameters.as_ref()?;
    match parameters {
        serde_json::Value::Object(_) => parameters.get(key)?.as_str().map(ToOwned::to_owned),
        serde_json::Value::String(raw_json) => serde_json::from_str::<serde_json::Value>(raw_json)
            .ok()?
            .get(key)?
            .as_str()
            .map(ToOwned::to_owned),
        _ => None,
    }
}

async fn parse_import_multipart(mut payload: Multipart) -> Result<ParsedImportMultipart, String> {
    let mut parsed = ParsedImportMultipart::default();
    let mut total_file_bytes = 0usize;

    while let Some(field_result) = payload.next().await {
        let mut field =
            field_result.map_err(|error| format!("Multipart parse error: {}", error))?;
        let Some(content_disposition) = field.content_disposition() else {
            continue;
        };
        let field_name = content_disposition.get_name().unwrap_or("").to_string();

        if matches!(field_name.as_str(), "file" | "zip") {
            let mut data = BytesMut::new();
            while let Some(chunk) = field.next().await {
                let bytes =
                    chunk.map_err(|error| format!("Failed to read import ZIP: {}", error))?;
                total_file_bytes = total_file_bytes
                    .checked_add(bytes.len())
                    .ok_or_else(|| "Import ZIP is too large".to_string())?;
                if total_file_bytes > MAX_IMPORT_ZIP_BYTES {
                    return Err(format!(
                        "Import ZIP exceeds maximum size of {} bytes",
                        MAX_IMPORT_ZIP_BYTES
                    ));
                }
                data.extend_from_slice(&bytes);
            }
            parsed.file = Some(data.freeze());
        } else if matches!(
            field_name.as_str(),
            "namespace_id" | "table_name" | "user_id"
        ) {
            let value = read_limited_text_field(&mut field, &field_name).await?;
            match field_name.as_str() {
                "namespace_id" => parsed.namespace_id = Some(value),
                "table_name" => parsed.table_name = Some(value),
                "user_id" => parsed.user_id = Some(value),
                _ => {},
            }
        }
    }

    Ok(parsed)
}

async fn read_limited_text_field(
    field: &mut actix_multipart::Field,
    field_name: &str,
) -> Result<String, String> {
    let mut data = BytesMut::new();
    while let Some(chunk) = field.next().await {
        let bytes =
            chunk.map_err(|error| format!("Failed to read field '{}': {}", field_name, error))?;
        let next_len = data
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| format!("Field '{}' is too large", field_name))?;
        if next_len > MAX_TEXT_FIELD_BYTES {
            return Err(format!(
                "Field '{}' exceeds maximum size of {} bytes",
                field_name, MAX_TEXT_FIELD_BYTES
            ));
        }
        data.extend_from_slice(&bytes);
    }

    Ok(String::from_utf8_lossy(&data).trim().to_string())
}

fn parse_transfer_request(
    request: &TableExportRequest,
    app_context: &AppContext,
) -> Result<(TableId, kalamdb_commons::schemas::TableType, Option<String>), String> {
    let namespace_id = NamespaceId::try_new(request.namespace_id.trim())
        .map_err(|error| format!("Invalid namespace_id: {}", error))?;
    let table_name = TableName::try_new(request.table_name.trim())
        .map_err(|error| format!("Invalid table_name: {}", error))?;
    let table_id = TableId::new(namespace_id, table_name);
    let schema_registry = app_context.schema_registry();
    let table_def = schema_registry
        .get_table_if_exists(&table_id)
        .map_err(|error| format!("Failed to resolve table '{}': {}", table_id.full_name(), error))?
        .ok_or_else(|| format!("Table not found: {}", table_id.full_name()))?;
    let table_type = table_def.table_type;
    let user_id = request
        .user_id
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    validate_table_transfer_scope(table_type, user_id.as_deref())
        .map_err(|error| error.to_string())?;

    Ok((table_id, table_type, user_id))
}

fn bad_request(message: String) -> HttpResponse {
    HttpResponse::BadRequest().json(serde_json::json!({
        "error": "invalid_input",
        "message": message,
    }))
}

fn forbidden(message: &str) -> HttpResponse {
    HttpResponse::Forbidden().json(serde_json::json!({
        "error": "permission_denied",
        "message": message,
    }))
}

fn internal_error(message: String) -> HttpResponse {
    HttpResponse::InternalServerError().json(serde_json::json!({
        "error": "internal_error",
        "message": message,
    }))
}
