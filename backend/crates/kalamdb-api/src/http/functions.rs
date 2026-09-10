//! POST /v1/functions/{namespace}/{procedure}

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse, Responder};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::{
    conversions::arrow_json_conversion::{scalar_value_to_js_json, scalar_value_to_json},
    models::{NamespaceId, RoutineId},
    KalamDataType,
};
use kalamdb_core::{
    app_context::AppContext,
    functions::{
        json_to_routine_value, FunctionCallOrigin, FunctionService, HttpResponseOverrides,
        RoutineValue,
    },
    sql::context::ExecutionContext,
};
use kalamdb_session::AuthSession;
use parking_lot::Mutex;
use serde_json::{json, Value};
use uuid::Uuid;

const REJECTED_CONTEXT_KEYS: &[&str] = &["context", "ctx", "source", "actor", "tx"];

pub async fn invoke_function_v1(
    extractor: AuthSessionExtractor,
    http_req: HttpRequest,
    path: web::Path<(String, String)>,
    body: web::Json<Value>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    let (namespace, procedure) = path.into_inner();
    let namespace_id = NamespaceId::new(namespace);
    let routine_id = RoutineId::from_parts(Some(&namespace_id), &procedure);

    if let Some(key) = rejected_context_key(&body) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "INVALID_ARGUMENTS",
            "message": format!("client-supplied context field '{key}' is not allowed"),
        }));
    }

    let stores = app_context.system_tables().catalog_stores();
    let parameters = match stores.list_parameters(&routine_id) {
        Ok(parameters) => parameters,
        Err(error) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "INTERNAL_RUNTIME_ERROR",
                "message": error.to_string(),
            }));
        },
    };
    let args = match bind_json_args(&body, &parameters) {
        Ok(args) => args,
        Err(message) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "INVALID_ARGUMENTS",
                "message": message,
            }));
        },
    };

    let mut headers = Vec::new();
    for (name, value) in http_req.headers() {
        if let Ok(value) = value.to_str() {
            headers.push((name.as_str().to_string(), value.to_string()));
        }
    }
    let query: Vec<(String, String)> = http_req
        .query_string()
        .split('&')
        .filter_map(|pair| {
            if pair.is_empty() {
                return None;
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((key.to_string(), value.to_string()))
        })
        .collect();
    let response = Arc::new(Mutex::new(HttpResponseOverrides::default()));
    let origin = FunctionCallOrigin::Http {
        method:   http_req.method().as_str().to_string(),
        path:     http_req.path().to_string(),
        headers:  Arc::new(headers),
        query:    Arc::new(query),
        response: Arc::clone(&response),
    };

    let exec_ctx =
        ExecutionContext::from_session(session, Arc::clone(&app_context.base_session_context()))
            .with_namespace_id(namespace_id)
            .with_request_id(Uuid::now_v7().to_string());

    match FunctionService::invoke(
        Arc::clone(app_context.get_ref()),
        &exec_ctx,
        origin,
        routine_id,
        args,
    )
    .await
    {
        Ok(result) => {
            let payload = match scalar_value_to_json(&result.value.value) {
                Ok(value) => value.0,
                Err(error) => {
                    return HttpResponse::InternalServerError().json(json!({
                        "status": "error",
                        "code": "INTERNAL_RUNTIME_ERROR",
                        "message": error.to_string(),
                    }));
                },
            };
            let status = result.http_status.unwrap_or(200);
            let mut builder = HttpResponse::build(
                actix_web::http::StatusCode::from_u16(status)
                    .unwrap_or(actix_web::http::StatusCode::OK),
            );
            for (name, value) in result.http_headers {
                builder.insert_header((name, value));
            }
            builder.json(payload)
        },
        Err(error) => {
            let (status, code) = function_http_status(&error);
            HttpResponse::build(status).json(json!({
                "status": "error",
                "code": code,
                "message": error.user_message(),
            }))
        },
    }
}

fn function_http_status(
    error: &kalamdb_core::error::KalamDbError,
) -> (actix_web::http::StatusCode, &'static str) {
    use actix_web::http::StatusCode;
    use kalamdb_functions::FunctionErrorCode;
    match error.function_error_code() {
        Some(FunctionErrorCode::ProcedureNotFound)
        | Some(FunctionErrorCode::ProcedureNotImplemented) => {
            (StatusCode::NOT_FOUND, error.function_error_code().unwrap().as_str())
        },
        Some(FunctionErrorCode::ExecuteDenied) => (StatusCode::FORBIDDEN, "EXECUTE_DENIED"),
        Some(FunctionErrorCode::AuthenticationRequired) => {
            (StatusCode::UNAUTHORIZED, "AUTHENTICATION_REQUIRED")
        },
        Some(FunctionErrorCode::InvalidArguments) => (StatusCode::BAD_REQUEST, "INVALID_ARGUMENTS"),
        Some(FunctionErrorCode::ResourceLimit) => (StatusCode::TOO_MANY_REQUESTS, "RESOURCE_LIMIT"),
        Some(FunctionErrorCode::ProcedureTimeout) => {
            (StatusCode::GATEWAY_TIMEOUT, "PROCEDURE_TIMEOUT")
        },
        Some(FunctionErrorCode::ContractMismatch)
        | Some(FunctionErrorCode::AbiMismatch)
        | Some(FunctionErrorCode::StaleRevision) => {
            (StatusCode::CONFLICT, error.function_error_code().unwrap().as_str())
        },
        Some(FunctionErrorCode::InternalRuntimeError) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_RUNTIME_ERROR")
        },
        None => (StatusCode::BAD_REQUEST, "INVALID_ARGUMENTS"),
    }
}

fn rejected_context_key(body: &Value) -> Option<&str> {
    let Value::Object(map) = body else {
        return None;
    };
    REJECTED_CONTEXT_KEYS.iter().copied().find(|key| map.contains_key(*key))
}

fn bind_json_args(
    body: &Value,
    parameters: &[kalamdb_system::CatalogRoutineParameter],
) -> Result<Vec<RoutineValue>, String> {
    match body {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| bind_json_value(item, parameters.get(index)))
            .collect(),
        Value::Object(map) => {
            if parameters.is_empty() {
                return Ok(Vec::new());
            }
            let mut args = Vec::with_capacity(parameters.len());
            for parameter in parameters {
                let value = map
                    .get(&parameter.name)
                    .ok_or_else(|| format!("missing procedure argument '{}'", parameter.name))?;
                args.push(bind_json_value(value, Some(parameter))?);
            }
            Ok(args)
        },
        other => Err(format!("function body must be a JSON object or array, got {other}")),
    }
}

fn bind_json_value(
    value: &Value,
    parameter: Option<&kalamdb_system::CatalogRoutineParameter>,
) -> Result<RoutineValue, String> {
    let routine = json_to_routine_value(value, parameter_data_type(parameter).as_ref())
        .map_err(|error| error.to_string())?;
    let json = scalar_value_to_js_json(&routine.value).map_err(|error| error.to_string())?;
    let bytes = kalamdb_serialization::encode_function_value("rest", &json.0)
        .map_err(|error| error.to_string())?;
    Ok(routine.with_transfer(bytes::Bytes::from(bytes), "rest"))
}

fn parameter_data_type(
    parameter: Option<&kalamdb_system::CatalogRoutineParameter>,
) -> Option<KalamDataType> {
    let parameter = parameter?;
    if parameter.is_array {
        return None;
    }
    parameter.builtin_data_type()
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

        let missing: KalamDbError = FunctionsError::NotImplemented("api.x".into()).into();
        let (status, code) = function_http_status(&missing);
        assert_eq!(status, actix_web::http::StatusCode::NOT_FOUND);
        assert_eq!(code, "PROCEDURE_NOT_IMPLEMENTED");
    }

    #[test]
    fn rest_json_encodes_function_transfer_once() {
        let value = serde_json::json!(7);
        let routine = super::bind_json_value(&value, None).unwrap();
        assert!(routine.transfer.is_some());
        assert_eq!(routine.contract_hash.as_deref(), Some("rest"));
        let decoded = kalamdb_serialization::decode_function_value(
            routine.transfer.as_ref().unwrap(),
            "rest",
        )
        .unwrap();
        assert_eq!(decoded, value);
    }
}
