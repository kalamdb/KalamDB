//! Admin CAS activate/rollback for project function modules.

use std::sync::Arc;

use actix_web::{web, HttpResponse, Responder};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::{FunctionModuleId, FunctionRevisionId};
use kalamdb_core::{
    app_context::AppContext,
    functions::{activate_module_artifact, rollback_module_revision},
};
use kalamdb_session::{is_admin_role, AuthSession};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct ActivateModuleRequest {
    pub artifact:      String,
    #[serde(rename = "contractHash")]
    pub contract_hash: String,
    #[serde(rename = "abiVersion")]
    pub abi_version:   u32,
    #[serde(default)]
    pub exports:       Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RollbackModuleRequest {
    #[serde(rename = "revisionId")]
    pub revision_id: String,
}

pub async fn activate_function_module_v1(
    extractor: AuthSessionExtractor,
    path: web::Path<String>,
    body: web::Json<ActivateModuleRequest>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    if !is_admin_role(session.role()) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "EXECUTE_DENIED",
            "message": "DBA or System role is required to activate function modules",
        }));
    }
    let module_id = FunctionModuleId::new(path.into_inner());
    match activate_module_artifact(
        app_context.get_ref().as_ref(),
        module_id,
        body.artifact.as_bytes(),
        &body.contract_hash,
        body.abi_version,
        &body.exports,
    )
    .await
    {
        Ok(outcome) => HttpResponse::Ok().json(json!({
            "status": "ok",
            "outcome": format!("{outcome:?}"),
        })),
        Err(error) => activation_error(error),
    }
}

pub async fn rollback_function_module_v1(
    extractor: AuthSessionExtractor,
    path: web::Path<String>,
    body: web::Json<RollbackModuleRequest>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    if !is_admin_role(session.role()) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "EXECUTE_DENIED",
            "message": "DBA or System role is required to roll back function modules",
        }));
    }
    let module_id = FunctionModuleId::new(path.into_inner());
    let revision_id = FunctionRevisionId::new(body.revision_id.clone());
    match rollback_module_revision(app_context.get_ref().as_ref(), module_id, revision_id).await {
        Ok(outcome) => HttpResponse::Ok().json(json!({
            "status": "ok",
            "outcome": format!("{outcome:?}"),
        })),
        Err(error) => activation_error(error),
    }
}

fn activation_error(error: kalamdb_core::error::KalamDbError) -> HttpResponse {
    let code = error
        .function_error_code()
        .map(|code| code.as_str())
        .unwrap_or("INTERNAL_RUNTIME_ERROR");
    let mut status = match code {
        "ABI_MISMATCH" | "CONTRACT_MISMATCH" | "INVALID_ARGUMENTS" => HttpResponse::BadRequest(),
        "STALE_REVISION" => HttpResponse::Conflict(),
        "PROCEDURE_NOT_FOUND" => HttpResponse::NotFound(),
        "EXECUTE_DENIED" => HttpResponse::Forbidden(),
        _ => HttpResponse::InternalServerError(),
    };
    status.json(json!({
        "status": "error",
        "code": code,
        "message": error.to_string(),
    }))
}
