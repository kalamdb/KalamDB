//! Shared HTTP status mapping for routine endpoints.

use actix_web::{HttpRequest, HttpResponse, http::StatusCode};
use kalamdb_core::error::KalamDbError;
use kalamdb_functions::FunctionErrorCode;
use kalamdb_serialization::DOMAIN_EPOCH;
use serde_json::json;

pub(super) fn function_http_status(error: &KalamDbError) -> (StatusCode, &'static str) {
    if let KalamDbError::Coded(coded) = error {
        let leaf = coded.leaf_code();
        if let Some(code) = leaf.client_code() {
            let status =
                StatusCode::from_u16(leaf.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
            return (status, code);
        }
    }
    let Some(code) = error.function_error_code() else {
        return (StatusCode::BAD_REQUEST, "INVALID_ARGUMENTS");
    };
    match code {
        FunctionErrorCode::ProcedureNotFound | FunctionErrorCode::ProcedureNotImplemented => {
            (StatusCode::NOT_FOUND, code.as_str())
        },
        FunctionErrorCode::ExecuteDenied => (StatusCode::FORBIDDEN, code.as_str()),
        FunctionErrorCode::AuthenticationRequired => (StatusCode::UNAUTHORIZED, code.as_str()),
        FunctionErrorCode::InvalidArguments => (StatusCode::BAD_REQUEST, code.as_str()),
        FunctionErrorCode::ResourceLimit => (StatusCode::TOO_MANY_REQUESTS, code.as_str()),
        FunctionErrorCode::ProcedureTimeout => (StatusCode::GATEWAY_TIMEOUT, code.as_str()),
        FunctionErrorCode::ContractMismatch
        | FunctionErrorCode::AbiMismatch
        | FunctionErrorCode::StaleRevision => (StatusCode::CONFLICT, code.as_str()),
        FunctionErrorCode::InternalRuntimeError => {
            (StatusCode::INTERNAL_SERVER_ERROR, code.as_str())
        },
    }
}

pub(super) fn check_epoch_header(http_req: &HttpRequest) -> Option<HttpResponse> {
    let Some(raw_epoch) = http_req
        .headers()
        .get("x-kalam-epoch")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
    else {
        return None;
    };
    if raw_epoch.parse::<u16>().ok() == Some(DOMAIN_EPOCH) {
        return None;
    }
    Some(HttpResponse::Conflict().json(json!({
        "status": "error",
        "code": "EPOCH_MISMATCH",
        "message": format!(
            "unsupported protocol epoch '{raw_epoch}'; current server epoch is {DOMAIN_EPOCH}"
        ),
    })))
}

#[cfg(test)]
mod tests {
    use kalamdb_core::error::KalamDbError;
    use kalamdb_functions::{FunctionErrorCode, FunctionsError};

    use super::function_http_status;

    #[test]
    fn rest_maps_typed_codes_not_message_substrings() {
        let denied: KalamDbError = FunctionsError::ExecuteDenied("api.x".into()).into();
        let (status, code) = function_http_status(&denied);
        assert_eq!(status, actix_web::http::StatusCode::FORBIDDEN);
        assert_eq!(code, "EXECUTE_DENIED");
        assert_eq!(denied.function_error_code(), Some(FunctionErrorCode::ExecuteDenied));

        let missing: KalamDbError = FunctionsError::UnknownProcedure("api.x".into()).into();
        let (status, code) = function_http_status(&missing);
        assert_eq!(status, actix_web::http::StatusCode::NOT_FOUND);
        assert_eq!(code, "PROCEDURE_NOT_FOUND");

        let unimplemented: KalamDbError = FunctionsError::NotImplemented("api.x".into()).into();
        let (status, code) = function_http_status(&unimplemented);
        assert_eq!(status, actix_web::http::StatusCode::NOT_FOUND);
        assert_eq!(code, "PROCEDURE_NOT_IMPLEMENTED");

        let invalid: KalamDbError =
            FunctionsError::Invalid("nested procedures cannot mutate ctx.http".into()).into();
        let (status, code) = function_http_status(&invalid);
        assert_eq!(status, actix_web::http::StatusCode::BAD_REQUEST);
        assert_eq!(code, "INVALID_ARGUMENTS");

        let js: KalamDbError = FunctionsError::Javascript("boom".into()).into();
        let (status, code) = function_http_status(&js);
        assert_eq!(status, actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(code, "INTERNAL_RUNTIME_ERROR");
    }
}
