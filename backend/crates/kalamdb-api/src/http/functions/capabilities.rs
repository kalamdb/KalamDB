//! `GET /v1/api/capabilities`

use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, web};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_core::app_context::AppContext;
use kalamdb_serialization::{DOMAIN_CODEC_VERSION, DOMAIN_EPOCH};
use kalamdb_session::AuthSession;
use serde_json::json;

use super::error::check_epoch_header;

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
                return HttpResponse::Conflict().json(json!({
                    "status": "error",
                    "code": "EPOCH_MISMATCH",
                    "message": format!(
                        "unsupported requested epoch '{value}'; current server epoch is {DOMAIN_EPOCH}"
                    ),
                }));
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
    if http_req
        .headers()
        .get(actix_web::http::header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str())
    {
        return HttpResponse::NotModified()
            .insert_header((actix_web::http::header::ETAG, etag))
            .finish();
    }
    HttpResponse::Ok()
        .insert_header((actix_web::http::header::ETAG, etag))
        .insert_header((
            actix_web::http::header::CACHE_CONTROL,
            "private, max-age=0, must-revalidate",
        ))
        .json(payload)
}
