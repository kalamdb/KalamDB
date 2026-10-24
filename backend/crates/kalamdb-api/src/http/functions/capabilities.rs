//! `GET /v1/api/capabilities`

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_core::app_context::AppContext;
use kalamdb_serialization::{DOMAIN_CODEC_VERSION, DOMAIN_EPOCH};
use kalamdb_session::AuthSession;
use serde_json::json;

use super::error::{check_epoch_header, function_error_response, private_cached_json};

/// Authorized current-epoch capability endpoint.
pub async fn capabilities_v1(
    extractor: AuthSessionExtractor,
    http_req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
) -> HttpResponse {
    let _session: AuthSession = extractor.into();
    if let Some(err_resp) = check_epoch_header(&http_req) {
        return err_resp;
    }
    for pair in http_req.query_string().split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key == "epoch" && value.parse::<u16>().ok() != Some(DOMAIN_EPOCH) {
                return function_error_response(
                    actix_web::http::StatusCode::CONFLICT,
                    "EPOCH_MISMATCH",
                    format!(
                        "unsupported requested epoch '{value}'; current server epoch is \
                         {DOMAIN_EPOCH}"
                    ),
                );
            }
        }
    }
    let active_gen = app_context.function_runtime().active_set().generation;
    let payload = json!({
        "epoch": DOMAIN_EPOCH,
        "codecVersion": DOMAIN_CODEC_VERSION,
        "codec": "kalam-packed",
        "moduleGeneration": active_gen,
        "features": [
            "binary_rpc",
            "typed_json_rpc",
            "binary_ws",
            "scoped_etags",
            "idempotency_receipts"
        ],
        "limits": {
            "maxFrameWireBytes": kalamdb_serialization::DEFAULT_MAX_FRAME_WIRE_BYTES,
            "maxDecodedBytes": kalamdb_serialization::DEFAULT_MAX_DOMAIN_DECODED_BYTES,
            "maxElements": kalamdb_serialization::DEFAULT_MAX_DOMAIN_ELEMENTS,
            "maxDepth": kalamdb_serialization::DEFAULT_MAX_DOMAIN_DEPTH,
        }
    });
    let etag = format!("\"cap-epoch{DOMAIN_EPOCH}-gen{active_gen}\"");
    private_cached_json(&http_req, etag, payload)
}
