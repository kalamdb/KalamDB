//! POST /v1/functions/{namespace}/{procedure}

use std::{collections::HashMap, sync::Arc};

use actix_web::{HttpRequest, HttpResponse, Responder, web};
use bytes::Bytes;
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::{
    KalamDataType,
    conversions::arrow_json_conversion::scalar_value_to_json,
    models::{RoutineId, SchemaId},
};
use kalamdb_core::{
    app_context::AppContext,
    functions::{
        FunctionCallOrigin, FunctionService, HttpResponseOverrides, RoutineValue,
        bind_domain_call_arguments, json_to_routine_value, resolve_pinned_descriptor,
        routine_value_as_json, routine_value_to_domain_value,
    },
    sql::context::ExecutionContext,
};
use kalamdb_serialization::{
    DOMAIN_EPOCH, DOMAIN_MAGIC, DomainDecodeBudget, DomainMessageKind, EndpointSurface,
    SerializationError, decode_domain_frame_for_surface, encode_domain_frame,
};
use kalamdb_session::AuthSession;
use parking_lot::Mutex;
use serde_json::{Value, json};
use uuid::Uuid;

const REJECTED_CONTEXT_KEYS: &[&str] = &["context", "ctx", "source", "actor", "tx"];
pub const CONTENT_TYPE_KALAM_DOMAIN: &str = "application/x-kalam-domain";

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
    let is_binary = content_type.eq_ignore_ascii_case(CONTENT_TYPE_KALAM_DOMAIN)
        || content_type.eq_ignore_ascii_case("application/octet-stream")
        || body.starts_with(&DOMAIN_MAGIC);

    let stores = app_context.system_tables().catalog_stores();
    let module_gen = app_context.function_runtime().active_set().generation;

    let (args, return_data_type) = if is_binary {
        let frame = match decode_domain_frame_for_surface(
            &body,
            EndpointSurface::PublicRpc,
            DomainDecodeBudget::default(),
        ) {
            Ok(frame) => frame,
            Err(SerializationError::EpochMismatch { expected, found }) => {
                return HttpResponse::Conflict().json(json!({
                    "status": "error",
                    "code": "EPOCH_MISMATCH",
                    "message": format!("unsupported protocol epoch {found}; server epoch is {expected}"),
                }));
            },
            Err(SerializationError::UnsupportedFormat(msg)) => {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "code": "UNSUPPORTED_FORMAT",
                    "message": msg,
                }));
            },
            Err(SerializationError::ForbiddenMessageKind { kind, surface }) => {
                return HttpResponse::Forbidden().json(json!({
                    "status": "error",
                    "code": "FORBIDDEN_MESSAGE_KIND",
                    "message": format!("message kind 0x{kind:02x} is forbidden on {surface}"),
                }));
            },
            Err(err) => {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "code": "INVALID_ARGUMENTS",
                    "message": err.to_string(),
                }));
            },
        };
        if frame.kind != DomainMessageKind::RpcRequest {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "INVALID_ARGUMENTS",
                "message": format!("expected RpcRequest frame kind, got {:?}", frame.kind),
            }));
        }
        let descriptor = match resolve_pinned_descriptor(&stores, &routine_id, module_gen) {
            Ok(desc) => desc,
            Err(error) => {
                let (status, code) = super::error::function_http_status(&error);
                return HttpResponse::build(status).json(json!({
                    "status": "error",
                    "code": code,
                    "message": error.user_message(),
                }));
            },
        };
        if let Some(expected_rev) = http_req
            .headers()
            .get("x-kalam-descriptor-revision")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
        {
            if expected_rev != descriptor.revision_hex {
                return HttpResponse::Conflict().json(json!({
                    "status": "error",
                    "code": "STALE_REVISION",
                    "message": format!(
                        "descriptor revision mismatch for {}: client {}, server {}",
                        routine_id, expected_rev, descriptor.revision_hex
                    ),
                }));
            }
        }
        let bound = match bind_domain_call_arguments(&frame.payload, &descriptor) {
            Ok(args) => args,
            Err(error) => {
                let (status, code) = super::error::function_http_status(&error);
                return HttpResponse::build(status).json(json!({
                    "status": "error",
                    "code": code,
                    "message": error.user_message(),
                }));
            },
        };
        (bound, descriptor.routine.return_data_type)
    } else {
        let json_body: Value = match serde_json::from_slice(&body) {
            Ok(val) => val,
            Err(err) => {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "code": "INVALID_ARGUMENTS",
                    "message": format!("invalid JSON request body: {err}"),
                }));
            },
        };
        let unwrapped_body =
            match unwrap_typed_json_envelope(&json_body, &stores, &routine_id, module_gen) {
                Ok(val) => val,
                Err(resp) => return resp,
            };
        let parameters = {
            let _span = kalamdb_observability::kdb_info_span_entered!("func.http.list_params");
            match stores.list_parameters(&routine_id) {
                Ok(parameters) => parameters,
                Err(error) => {
                    return HttpResponse::InternalServerError().json(json!({
                        "status": "error",
                        "code": "INTERNAL_RUNTIME_ERROR",
                        "message": error.to_string(),
                    }));
                },
            }
        };
        let args = {
            let _span = kalamdb_observability::kdb_info_span_entered!("func.http.bind_args");
            match bind_json_args(&unwrapped_body, &parameters) {
                Ok(args) => args,
                Err(message) => {
                    return HttpResponse::BadRequest().json(json!({
                        "status": "error",
                        "code": "INVALID_ARGUMENTS",
                        "message": message,
                    }));
                },
            }
        };
        (args, None)
    };

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

        let exec_ctx = ExecutionContext::from_session(
            session,
            Arc::clone(&app_context.base_session_context()),
        )
        .with_namespace_id(namespace_id)
        .with_request_id(Uuid::now_v7().to_string());
        (origin, exec_ctx)
    };

    match kalamdb_observability::kdb_await_in_info_span!(
        FunctionService::invoke(
            Arc::clone(app_context.get_ref()),
            &exec_ctx,
            origin,
            routine_id,
            args,
        ),
        "func.invoke"
    ) {
        Ok(result) => {
            let _span = kalamdb_observability::kdb_info_span_entered!("func.http.encode_response");
            if is_binary {
                let domain_val =
                    match routine_value_to_domain_value(&result.value, return_data_type.as_ref()) {
                        Ok(v) => v,
                        Err(err) => {
                            return HttpResponse::InternalServerError().json(json!({
                                "status": "error",
                                "code": "INTERNAL_RUNTIME_ERROR",
                                "message": err.to_string(),
                            }));
                        },
                    };
                let frame_bytes =
                    match encode_domain_frame(DomainMessageKind::RpcResult, 0, 0, &domain_val) {
                        Ok(b) => b,
                        Err(err) => {
                            return HttpResponse::InternalServerError().json(json!({
                                "status": "error",
                                "code": "INTERNAL_RUNTIME_ERROR",
                                "message": err.to_string(),
                            }));
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
                return builder.body(frame_bytes);
            }
            let payload = match function_http_payload(&result.value) {
                Ok(value) => value,
                Err(error) => {
                    return HttpResponse::InternalServerError().json(json!({
                        "status": "error",
                        "code": "INTERNAL_RUNTIME_ERROR",
                        "message": error,
                    }));
                },
            };
            function_success_response(payload, result.http_status, result.http_headers)
        },
        Err(error) => {
            let (status, code) = super::error::function_http_status(&error);
            HttpResponse::build(status).json(json!({
                "status": "error",
                "code": code,
                "message": error.user_message(),
            }))
        },
    }
}

fn rejected_context_key(body: &Value) -> Option<&str> {
    let Value::Object(map) = body else {
        return None;
    };
    REJECTED_CONTEXT_KEYS.iter().copied().find(|key| map.contains_key(*key))
}

fn unwrap_typed_json_envelope(
    body: &Value,
    stores: &kalamdb_system::CatalogStores,
    routine_id: &RoutineId,
    module_gen: u64,
) -> Result<Value, HttpResponse> {
    if let Value::Object(map) = body {
        if map.contains_key("epoch") && map.contains_key("arguments") {
            let epoch = map.get("epoch").and_then(Value::as_u64);
            if epoch != Some(u64::from(DOMAIN_EPOCH)) {
                return Err(HttpResponse::Conflict().json(json!({
                    "status": "error",
                    "code": "EPOCH_MISMATCH",
                    "message": format!(
                        "unsupported envelope epoch {:?}; current server epoch is {DOMAIN_EPOCH}",
                        map.get("epoch")
                    ),
                })));
            }
            if let Some(key) =
                REJECTED_CONTEXT_KEYS.iter().copied().find(|key| map.contains_key(*key))
            {
                return Err(HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "code": "INVALID_ARGUMENTS",
                    "message": format!("client-supplied context field '{key}' is not allowed"),
                })));
            }
            if let Some(rev) = map.get("revision").and_then(Value::as_str).filter(|s| !s.is_empty())
            {
                if let Ok(desc) = resolve_pinned_descriptor(stores, routine_id, module_gen) {
                    if desc.revision_hex != rev {
                        return Err(HttpResponse::Conflict().json(json!({
                            "status": "error",
                            "code": "STALE_REVISION",
                            "message": format!(
                                "descriptor revision mismatch for {routine_id}: client {rev}, server {}",
                                desc.revision_hex
                            ),
                        })));
                    }
                }
            }
            return Ok(map.get("arguments").cloned().unwrap_or(Value::Null));
        }
    }
    if let Some(key) = rejected_context_key(body) {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "INVALID_ARGUMENTS",
            "message": format!("client-supplied context field '{key}' is not allowed"),
        })));
    }
    Ok(body.clone())
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
            if is_whole_body_json(map, parameters) {
                let body = Value::Object(map.clone());
                return Ok(vec![bind_json_value(&body, Some(&parameters[0]))?]);
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

fn function_success_response(
    payload: Value,
    status: Option<u16>,
    headers: HashMap<String, String>,
) -> HttpResponse {
    let body = match serde_json::to_vec(&payload) {
        Ok(bytes) => bytes,
        Err(error) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "INTERNAL_RUNTIME_ERROR",
                "message": error.to_string(),
            }));
        },
    };
    let status_code = actix_web::http::StatusCode::from_u16(status.unwrap_or(200))
        .unwrap_or(actix_web::http::StatusCode::OK);
    let mut builder = HttpResponse::build(status_code);
    let mut has_content_type = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        builder.insert_header((name, value));
    }
    if !has_content_type {
        builder.insert_header((actix_web::http::header::CONTENT_TYPE, "application/json"));
    }
    builder.body(body)
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
        KalamDataType,
        models::{RoutineId, RoutineParameterId, SchemaId},
    };
    use kalamdb_core::functions::routine_value_as_json;
    use kalamdb_system::CatalogRoutineParameter;
    use serde_json::json;

    use super::{bind_json_args, function_http_payload, function_success_response};

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
        assert!(routine.transfer.is_none());
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
        let args = bind_json_args(&payload, &parameters).expect("whole-body JSON bind");
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
        let args = bind_json_args(&payload, &parameters).expect("named JSON bind");
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
        let response = function_success_response(json!({"ok": true}), Some(201), headers);
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

        let response = function_success_response(json!("echo"), None, HashMap::new());
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
