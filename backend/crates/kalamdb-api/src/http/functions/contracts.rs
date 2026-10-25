//! `GET /v1/api/contracts`

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_core::{app_context::AppContext, functions::resolve_pinned_descriptor};
use kalamdb_serialization::{sha256_hex, DOMAIN_CODEC_VERSION, DOMAIN_EPOCH};
use kalamdb_session::AuthSession;
use serde_json::{json, Value};

use super::error::{check_epoch_header, function_error_response, private_cached_json};

/// Authorized current-epoch contract manifest.
///
/// Publishes routine descriptors, parameter signatures, and scoped revisions without private
/// routine body source or secrets.
pub async fn contracts_manifest_v1(
    extractor: AuthSessionExtractor,
    http_req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
) -> HttpResponse {
    let _session: AuthSession = extractor.into();
    if let Some(err_resp) = check_epoch_header(&http_req) {
        return err_resp;
    }
    let stores = app_context.system_tables().catalog_stores();
    let module_gen = app_context.function_runtime().active_set().generation;
    let routines = match stores.list_routines() {
        Ok(routines) => routines,
        Err(err) => {
            return function_error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_RUNTIME_ERROR",
                err,
            );
        },
    };
    let mut operations = Vec::new();
    let mut etag_buf = Vec::with_capacity(128);
    etag_buf.extend_from_slice(&DOMAIN_EPOCH.to_le_bytes());
    etag_buf.extend_from_slice(&module_gen.to_le_bytes());
    for routine in routines {
        if let Ok(descriptor) = resolve_pinned_descriptor(&stores, &routine.routine_id) {
            etag_buf.extend_from_slice(&descriptor.revision_hash);
            let params: Vec<Value> = descriptor
                .parameters
                .iter()
                .map(|parameter| {
                    json!({
                        "name": parameter.name,
                        "ordinal": parameter.ordinal,
                        "typeName": parameter.type_name,
                        "typeId": parameter.type_id,
                        "isArray": parameter.is_array,
                        "notNull": parameter.not_null,
                        "mode": parameter.mode.as_str(),
                        "hasDefault": parameter.default_expr.is_some(),
                    })
                })
                .collect();
            operations.push(json!({
                "routineId": descriptor.routine_id.as_str(),
                "namespace": descriptor.routine.namespace_id.as_str(),
                "name": descriptor.routine.name,
                "kind": descriptor.routine.kind.as_str(),
                "security": descriptor.routine.security.as_str(),
                "volatility": descriptor.routine.volatility.as_str(),
                "revision": descriptor.revision_hex,
                "parameters": params,
                "returnType": descriptor.routine.return_type_name,
                "returnIsArray": descriptor.routine.return_is_array,
                "returnNotNull": descriptor.routine.return_not_null,
            }));
        }
    }
    let digest = sha256_hex(&etag_buf);
    let etag = format!("\"contracts-epoch{DOMAIN_EPOCH}-{}\"", &digest[..16]);
    private_cached_json(
        &http_req,
        etag,
        json!({
            "epoch": DOMAIN_EPOCH,
            "codecVersion": DOMAIN_CODEC_VERSION,
            "contractDigest": digest,
            "operations": operations,
        }),
    )
}
