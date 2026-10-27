//! POST /v1/functions/{namespace}/{procedure}

use std::{collections::HashMap, sync::Arc};

use actix_web::{web, HttpRequest, HttpResponse, Responder};
use bytes::Bytes;
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::{
    conversions::arrow_json_conversion::scalar_value_to_json,
    models::{RoutineId, SchemaId},
    KalamDataType,
};
use kalamdb_core::{
    app_context::AppContext,
    functions::{
        bind_domain_call_arguments_typed, json_to_routine_value, resolve_pinned_descriptor,
        routine_value_as_json, routine_value_to_domain_value, FunctionCallOrigin, FunctionService,
        HttpResponseOverrides, RoutineValue,
    },
    sql::context::ExecutionContext,
};
use kalamdb_serialization::{
    decode_domain_frame_for_surface, encode_domain_frame, DomainDecodeBudget, DomainMessageKind,
    EndpointSurface, SerializationError, DOMAIN_EPOCH,
};
use kalamdb_session::AuthSession;
use parking_lot::Mutex;
use serde_json::Value;
use uuid::Uuid;

const REJECTED_CONTEXT_KEYS: &[&str] = &["context", "ctx", "source", "actor", "tx"];
pub const CONTENT_TYPE_KALAM_DOMAIN: &str = "application/x-kalam-domain";

fn error_response(
    status: actix_web::http::StatusCode,
    code: &str,
    message: impl std::fmt::Display,
) -> HttpResponse {
    super::error::function_error_response(status, code, message)
}

pub async fn invoke_function_v1(
    extractor: AuthSessionExtractor,
    http_req: HttpRequest,
    path: web::Path<(String, String)>,
    body: Bytes,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();
    let (namespace, procedure) = path.into_inner();
    let namespace_id = SchemaId::new(namespace);
    let routine_id = RoutineId::from_parts(Some(&namespace_id), &procedure);

    if let Some(err_resp) = super::error::check_epoch_header(&http_req) {
        return err_resp;
    }

    let content_type = http_req
        .headers()
        .get(actix_web::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");
    let request_binary = content_type_is_domain(content_type);
    let response_binary = accept_prefers_domain(&http_req).unwrap_or(request_binary);

    let idempotency_key = http_req
        .headers()
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty());
    if let Some(key) = idempotency_key {
        match super::idempotency::lookup(
            session.user_id().as_str(),
            routine_id.as_str(),
            key,
            &body,
        ) {
            super::idempotency::IdempotencyLookup::Replay(stored) => {
                return stored.into_response();
            },
            super::idempotency::IdempotencyLookup::Conflict => {
                return error_response(
                    actix_web::http::StatusCode::CONFLICT,
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used with a different request body",
                );
            },
            super::idempotency::IdempotencyLookup::Fresh => {},
        }
    }

    let stores = app_context.system_tables().catalog_stores();
    let descriptor = match resolve_pinned_descriptor(&stores, &routine_id) {
        Ok(desc) => desc,
        Err(error) => {
            let (status, code) = super::error::function_http_status(&error);
            return error_response(status, code, error.user_message());
        },
    };
    if let Some(response) = revision_mismatch(
        &routine_id,
        &descriptor.revision_hex,
        http_req
            .headers()
            .get("x-kalam-descriptor-revision")
            .and_then(|value| value.to_str().ok()),
    ) {
        return response;
    }

    let return_data_type = descriptor.routine.return_data_type;
    let args = if request_binary {
        let frame = match decode_domain_frame_for_surface(
            &body,
            EndpointSurface::PublicRpc,
            DomainDecodeBudget::default(),
        ) {
            Ok(frame) => frame,
            Err(SerializationError::EpochMismatch { expected, found }) => {
                return error_response(
                    actix_web::http::StatusCode::CONFLICT,
                    "EPOCH_MISMATCH",
                    format!("unsupported protocol epoch {found}; server epoch is {expected}"),
                );
            },
            Err(SerializationError::UnsupportedFormat(msg)) => {
                return error_response(
                    actix_web::http::StatusCode::BAD_REQUEST,
                    "UNSUPPORTED_FORMAT",
                    msg,
                );
            },
            Err(SerializationError::ForbiddenMessageKind { kind, surface }) => {
                return error_response(
                    actix_web::http::StatusCode::FORBIDDEN,
                    "FORBIDDEN_MESSAGE_KIND",
                    format!("message kind 0x{kind:02x} is forbidden on {surface}"),
                );
            },
            Err(err) => {
                return error_response(
                    actix_web::http::StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENTS",
                    err,
                );
            },
        };
        if frame.kind != DomainMessageKind::RpcRequest {
            return error_response(
                actix_web::http::StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENTS",
                format!("expected RpcRequest frame kind, got {:?}", frame.kind),
            );
        }
        match bind_domain_call_arguments_typed(
            &frame.payload,
            &descriptor,
            app_context.schema_registry().type_registry(),
            &stores,
        ) {
            Ok(args) => args,
            Err(error) => {
                let (status, code) = super::error::function_http_status(&error);
                return error_response(status, code, error.user_message());
            },
        }
    } else {
        let json_body: Value = match serde_json::from_slice(&body) {
            Ok(val) => val,
            Err(err) => {
                return error_response(
                    actix_web::http::StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENTS",
                    format!("invalid JSON request body: {err}"),
                );
            },
        };
        let unwrapped_body = match unwrap_typed_json_envelope(json_body, &routine_id, &descriptor) {
            Ok(val) => val,
            Err(resp) => return resp,
        };
        match bind_json_args(unwrapped_body, &descriptor.input_parameters) {
            Ok(args) => args,
            Err(message) => {
                return error_response(
                    actix_web::http::StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENTS",
                    message,
                );
            },
        }
    };

    let user_id = session.user_id().as_str().to_string();
    let idempotency_key = idempotency_key.map(str::to_string);
    let (origin, exec_ctx) = {
        let _span = kalamdb_observability::kdb_info_span_entered!("func.http.context");
        let mut headers = Vec::new();
        for (name, value) in http_req.headers() {
            if is_sensitive_request_header(name.as_str()) {
                continue;
            }
            if let Ok(value) = value.to_str() {
                headers.push((name.as_str().to_string(), value.to_string()));
            }
        }
        let query = decode_query(http_req.query_string());
        let response = Arc::new(Mutex::new(HttpResponseOverrides::default()));
        let origin = FunctionCallOrigin::Http {
            method: http_req.method().as_str().to_string(),
            path: http_req.path().to_string(),
            headers: Arc::new(headers),
            query: Arc::new(query),
            response: Arc::clone(&response),
        };

        let exec_ctx = ExecutionContext::from_session(
            session,
            Arc::clone(&app_context.base_session_context()),
        )
        .with_namespace_id(namespace_id)
        .with_request_id(Uuid::now_v7().to_string());
        (origin, exec_ctx)
    };

    match kalamdb_observability::kdb_await_in_info_span!(
        FunctionService::invoke_pinned(
            Arc::clone(app_context.get_ref()),
            &exec_ctx,
            origin,
            Arc::clone(&descriptor),
            args,
        ),
        "func.invoke"
    ) {
        Ok(result) => {
            let _span = kalamdb_observability::kdb_info_span_entered!("func.http.encode_response");
            if response_binary {
                let domain_val =
                    match routine_value_to_domain_value(&result.value, return_data_type.as_ref()) {
                        Ok(v) => v,
                        Err(err) => {
                            return error_response(
                                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                                "INTERNAL_RUNTIME_ERROR",
                                err,
                            );
                        },
                    };
                let frame_bytes =
                    match encode_domain_frame(DomainMessageKind::RpcResult, 0, 0, &domain_val) {
                        Ok(b) => b,
                        Err(err) => {
                            return error_response(
                                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                                "INTERNAL_RUNTIME_ERROR",
                                err,
                            );
                        },
                    };
                let status_code =
                    actix_web::http::StatusCode::from_u16(result.http_status.unwrap_or(200))
                        .unwrap_or(actix_web::http::StatusCode::OK);
                let mut builder = HttpResponse::build(status_code);
                for (name, value) in result.http_headers {
                    builder.insert_header((name, value));
                }
                builder.insert_header((
                    actix_web::http::header::CONTENT_TYPE,
                    CONTENT_TYPE_KALAM_DOMAIN,
                ));
                remember_idempotency(
                    &user_id,
                    routine_id.as_str(),
                    idempotency_key.as_deref(),
                    &body,
                    status_code.as_u16(),
                    CONTENT_TYPE_KALAM_DOMAIN,
                    &frame_bytes,
                );
                return builder.body(frame_bytes);
            }
            if let Some(json_text) = result.value.json_sql_text() {
                let bytes = json_text.as_bytes().to_vec();
                let status = result.http_status.unwrap_or(200);
                remember_idempotency(
                    &user_id,
                    routine_id.as_str(),
                    idempotency_key.as_deref(),
                    &body,
                    status,
                    "application/json",
                    &bytes,
                );
                return bytes_response(status, "application/json", bytes, result.http_headers);
            }
            let payload = match function_http_payload(&result.value) {
                Ok(value) => value,
                Err(error) => {
                    return error_response(
                        actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "INTERNAL_RUNTIME_ERROR",
                        error,
                    );
                },
            };
            let bytes = match serde_json::to_vec(&payload) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return error_response(
                        actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "INTERNAL_RUNTIME_ERROR",
                        error,
                    );
                },
            };
            let status = result.http_status.unwrap_or(200);
            remember_idempotency(
                &user_id,
                routine_id.as_str(),
                idempotency_key.as_deref(),
                &body,
                status,
                "application/json",
                &bytes,
            );
            bytes_response(status, "application/json", bytes, result.http_headers)
        },
        Err(error) => {
            let (status, code) = super::error::function_http_status(&error);
            let message = error.user_message();
            let payload = serde_json::json!({
                "status": "error",
                "code": code,
                "message": message,
            });
            let bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
            remember_idempotency(
                &user_id,
                routine_id.as_str(),
                idempotency_key.as_deref(),
                &body,
                status.as_u16(),
                "application/json",
                &bytes,
            );
            error_response(status, code, message)
        },
    }
}

fn content_type_is_domain(content_type: &str) -> bool {
    let mime = content_type.split(';').next().unwrap_or(content_type).trim();
    mime.eq_ignore_ascii_case(CONTENT_TYPE_KALAM_DOMAIN)
        || mime.eq_ignore_ascii_case("application/octet-stream")
}

fn accept_prefers_domain(http_req: &HttpRequest) -> Option<bool> {
    let accept = http_req.headers().get(actix_web::http::header::ACCEPT)?.to_str().ok()?;
    if accept.contains(CONTENT_TYPE_KALAM_DOMAIN) {
        Some(true)
    } else if accept.contains("application/json") {
        Some(false)
    } else {
        None
    }
}

fn revision_mismatch(
    routine_id: &RoutineId,
    server_revision: &str,
    client_revision: Option<&str>,
) -> Option<HttpResponse> {
    let client_revision = client_revision.filter(|value| !value.is_empty())?;
    if client_revision == server_revision {
        return None;
    }
    Some(error_response(
        actix_web::http::StatusCode::CONFLICT,
        "STALE_REVISION",
        format!(
            "descriptor revision mismatch for {routine_id}: client {client_revision}, server \
             {server_revision}"
        ),
    ))
}

fn decode_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            },
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            },
            byte => {
                out.push(byte);
                index += 1;
            },
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn remember_idempotency(
    user_id: &str,
    routine_id: &str,
    key: Option<&str>,
    request_body: &[u8],
    status: u16,
    content_type: &str,
    response_body: &[u8],
) {
    let Some(key) = key else {
        return;
    };
    super::idempotency::store(
        user_id,
        routine_id,
        key,
        request_body,
        super::idempotency::StoredResponse {
            status,
            content_type: content_type.to_string(),
            body: response_body.to_vec(),
        },
    );
}

fn bytes_response(
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    headers: HashMap<String, String>,
) -> HttpResponse {
    let status_code =
        actix_web::http::StatusCode::from_u16(status).unwrap_or(actix_web::http::StatusCode::OK);
    let mut builder = HttpResponse::build(status_code);
    let mut has_content_type = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        builder.insert_header((name, value));
    }
    if !has_content_type {
        builder.insert_header((actix_web::http::header::CONTENT_TYPE, content_type));
    }
    builder.body(body)
}

fn rejected_context_response(key: &str) -> HttpResponse {
    error_response(
        actix_web::http::StatusCode::BAD_REQUEST,
        "INVALID_ARGUMENTS",
        format!("client-supplied context field '{key}' is not allowed"),
    )
}

fn rejected_context_key(body: &Value) -> Option<&str> {
    let Value::Object(map) = body else {
        return None;
    };
    REJECTED_CONTEXT_KEYS.iter().copied().find(|key| map.contains_key(*key))
}

fn unwrap_typed_json_envelope(
    body: Value,
    routine_id: &RoutineId,
    descriptor: &kalamdb_core::functions::PinnedRoutineDescriptor,
) -> Result<Value, HttpResponse> {
    let Value::Object(mut map) = body else {
        return Ok(body);
    };
    if map.contains_key("epoch") && map.contains_key("arguments") {
        let epoch = map.get("epoch").and_then(Value::as_u64);
        if epoch != Some(u64::from(DOMAIN_EPOCH)) {
            return Err(error_response(
                actix_web::http::StatusCode::CONFLICT,
                "EPOCH_MISMATCH",
                format!(
                    "unsupported envelope epoch {:?}; current server epoch is {DOMAIN_EPOCH}",
                    map.get("epoch")
                ),
            ));
        }
        if let Some(key) = REJECTED_CONTEXT_KEYS.iter().copied().find(|key| map.contains_key(*key))
        {
            return Err(rejected_context_response(key));
        }
        if let Some(response) = revision_mismatch(
            routine_id,
            &descriptor.revision_hex,
            map.get("revision").and_then(Value::as_str),
        ) {
            return Err(response);
        }
        return Ok(map.remove("arguments").unwrap_or(Value::Null));
    }
    let body = Value::Object(map);
    if let Some(key) = rejected_context_key(&body) {
        return Err(rejected_context_response(key));
    }
    Ok(body)
}

fn bind_json_args(
    body: Value,
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
            if is_whole_body_json(&map, parameters) {
                return Ok(vec![bind_json_value(&Value::Object(map), Some(&parameters[0]))?]);
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

fn is_whole_body_json(
    map: &serde_json::Map<String, Value>,
    parameters: &[kalamdb_system::CatalogRoutineParameter],
) -> bool {
    let [parameter] = parameters else {
        return false;
    };
    if parameter.is_array {
        return false;
    }
    parameter.builtin_data_type() == Some(KalamDataType::Json) && !map.contains_key(&parameter.name)
}

fn function_http_payload(value: &RoutineValue) -> Result<Value, String> {
    if value.json_sql {
        return routine_value_as_json(value)
            .ok_or_else(|| "JSON procedure result is not valid JSON".to_string());
    }
    scalar_value_to_json(&value.value)
        .map(|value| value.0)
        .map_err(|error| error.to_string())
}

fn bind_json_value(
    value: &Value,
    parameter: Option<&kalamdb_system::CatalogRoutineParameter>,
) -> Result<RoutineValue, String> {
    json_to_routine_value(value, parameter_data_type(parameter).as_ref())
        .map_err(|error| error.to_string())
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

fn is_sensitive_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use kalamdb_commons::{
        models::{RoutineId, RoutineParameterId, SchemaId},
        KalamDataType,
    };
    use kalamdb_core::functions::routine_value_as_json;
    use kalamdb_system::CatalogRoutineParameter;
    use serde_json::json;

    use super::{bind_json_args, bytes_response, function_http_payload};

    fn json_parameter(name: &str) -> CatalogRoutineParameter {
        let namespace = SchemaId::new("app");
        let routine_id = RoutineId::from_parts(Some(&namespace), "send");
        CatalogRoutineParameter {
            parameter_id: RoutineParameterId::new(&routine_id, 0).expect("parameter id"),
            routine_id,
            name: name.to_string(),
            ordinal: 0,
            type_id: None,
            type_name: "JSON".to_string(),
            is_array: false,
            not_null: true,
            nonempty: false,
            data_type: Some(KalamDataType::Json),
            mode: kalamdb_commons::models::RoutineParameterMode::default(),
            default_expr: None,
        }
    }

    #[test]
    fn rest_json_reaches_v8_as_json_sql() {
        let value = serde_json::json!(7);
        let routine = super::bind_json_value(&value, None).unwrap();
        assert_eq!(routine_value_as_json(&routine), Some(value));
    }

    #[test]
    fn rest_object_reaches_v8_as_an_object() {
        let value = serde_json::json!({"city": "Paris"});
        let routine = super::bind_json_value(&value, None).unwrap();
        let decoded = routine_value_as_json(&routine).expect("json object");
        assert_eq!(decoded["city"], "Paris");
        assert!(decoded.is_object(), "REST composite args must reach V8 as objects: {decoded}");
    }

    #[test]
    fn rest_whole_body_binds_single_json_parameter() {
        let parameters = vec![json_parameter("body")];
        let payload = json!({
            "conversationId": "c1",
            "text": "hello"
        });
        let args = bind_json_args(payload, &parameters).expect("whole-body JSON bind");
        assert_eq!(args.len(), 1);
        assert!(args[0].json_sql);
        let decoded = routine_value_as_json(&args[0]).expect("json body");
        assert_eq!(decoded["conversationId"], "c1");
        assert_eq!(decoded["text"], "hello");
    }

    #[test]
    fn rest_named_json_wrap_still_binds_inner_object() {
        let parameters = vec![json_parameter("body")];
        let payload = json!({
            "body": { "conversationId": "c1", "text": "hello" }
        });
        let args = bind_json_args(payload, &parameters).expect("named JSON bind");
        let decoded = routine_value_as_json(&args[0]).expect("named json body");
        assert_eq!(decoded["conversationId"], "c1");
    }

    #[test]
    fn rest_json_return_encodes_object_not_string() {
        let value = kalamdb_core::functions::json_to_routine_value(
            &json!({"ok": true, "text": "hello"}),
            Some(&KalamDataType::Json),
        )
        .expect("typed JSON return");
        let payload = function_http_payload(&value).expect("HTTP JSON payload");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["text"], "hello");
        assert!(payload.is_object());
    }

    #[test]
    fn rest_success_keeps_js_status_headers_and_content_type() {
        use actix_web::http::header::CONTENT_TYPE;

        let mut headers = HashMap::new();
        headers.insert("x-kalam-trace".into(), "ok".into());
        headers.insert("location".into(), "/created".into());
        headers.insert("content-type".into(), "application/json".into());
        let response = bytes_response(
            201,
            "application/json",
            serde_json::to_vec(&json!({"ok": true})).expect("json"),
            headers,
        );
        assert_eq!(response.status(), actix_web::http::StatusCode::CREATED);
        assert_eq!(
            response.headers().get("x-kalam-trace").and_then(|value| value.to_str().ok()),
            Some("ok")
        );
        assert_eq!(
            response.headers().get("location").and_then(|value| value.to_str().ok()),
            Some("/created")
        );
        assert_eq!(
            response.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn rest_success_defaults_json_content_type() {
        use actix_web::http::header::CONTENT_TYPE;

        let response = bytes_response(
            200,
            "application/json",
            serde_json::to_vec(&json!("echo")).expect("json"),
            HashMap::new(),
        );
        assert_eq!(response.status(), actix_web::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn sensitive_request_headers_are_stripped() {
        assert!(super::is_sensitive_request_header("Authorization"));
        assert!(super::is_sensitive_request_header("Cookie"));
        assert!(super::is_sensitive_request_header("proxy-authorization"));
        assert!(!super::is_sensitive_request_header("x-client-version"));
    }
}
