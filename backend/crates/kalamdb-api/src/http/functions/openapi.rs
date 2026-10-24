//! Typed OpenAPI 3.1 document for current-epoch routine calls.
//!
//! `GET /v1/api/openapi.json` serializes these models. The handler does not
//! assemble the document with untyped JSON maps.

use std::{collections::BTreeMap, sync::Arc};

use actix_web::{web, HttpRequest, HttpResponse};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_core::app_context::AppContext;
use kalamdb_serialization::DOMAIN_EPOCH;
use kalamdb_session::AuthSession;
use serde::Serialize;

use super::error::{check_epoch_header, function_error_response};

const STRUCTURED_ERROR_REF: &str = "#/components/responses/StructuredError";
const JSON_VALUE_TYPES: &[&str] = &["object", "array", "string", "number", "boolean", "null"];
const INT64_TYPES: &[&str] = &["integer", "string"];
const NO_SCOPES: &[&str] = &[];

/// One input parameter published on a routine operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenApiInput {
    pub name:         String,
    pub sql_type:     String,
    pub is_array:     bool,
    pub not_null:     bool,
    pub default_expr: Option<String>,
}

impl OpenApiInput {
    fn is_required(&self) -> bool {
        self.not_null && self.default_expr.is_none()
    }
}

/// Catalog fields the OpenAPI document is allowed to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenApiRoutineSpec {
    pub routine_id: String,
    pub namespace:  String,
    pub name:       String,
    pub summary:    String,
    pub inputs:     Vec<OpenApiInput>,
}

/// OpenAPI 3.1 document for the routine HTTP API.
#[derive(Debug, Serialize)]
pub struct OpenApiDocument {
    pub openapi:    &'static str,
    pub info:       OpenApiInfo,
    pub paths:      BTreeMap<String, OpenApiPathItem>,
    pub components: OpenApiComponents,
}

impl OpenApiDocument {
    pub fn from_routines(routines: &[OpenApiRoutineSpec]) -> Self {
        let mut paths = BTreeMap::new();
        for routine in routines {
            paths.insert(
                format!("/v1/functions/{}/{}", routine.namespace, routine.name),
                OpenApiPathItem {
                    post: OpenApiOperation::for_routine(routine),
                },
            );
        }
        Self {
            openapi: "3.1.0",
            info: OpenApiInfo {
                title:   "KalamDB Routine & Contract API",
                version: format!("epoch-{DOMAIN_EPOCH}"),
            },
            paths,
            components: OpenApiComponents::standard(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OpenApiInfo {
    pub title:   &'static str,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct OpenApiPathItem {
    pub post: OpenApiOperation,
}

#[derive(Debug, Serialize)]
pub struct OpenApiOperation {
    #[serde(rename = "operationId")]
    pub operation_id: String,
    pub summary:      String,
    pub security:     Vec<BearerAuthRequirement>,
    #[serde(rename = "requestBody")]
    pub request_body: OpenApiRequestBody,
    pub responses:    RoutineResponses,
}

impl OpenApiOperation {
    fn for_routine(routine: &OpenApiRoutineSpec) -> Self {
        let mut properties = BTreeMap::new();
        let mut required = Vec::new();
        for input in &routine.inputs {
            if input.is_required() {
                required.push(input.name.clone());
            }
            properties.insert(input.name.clone(), schema_for_input(input));
        }
        Self {
            operation_id: routine.routine_id.clone(),
            summary:      routine.summary.clone(),
            security:     vec![BearerAuthRequirement {
                bearer_auth: NO_SCOPES,
            }],
            request_body: OpenApiRequestBody {
                required: !required.is_empty(),
                content:  RoutineRequestContent {
                    json:   OpenApiMediaType {
                        schema: object_schema(properties, required),
                    },
                    domain: OpenApiMediaType {
                        schema: binary_schema(),
                    },
                },
            },
            responses:    RoutineResponses::standard(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BearerAuthRequirement {
    #[serde(rename = "bearerAuth")]
    pub bearer_auth: &'static [&'static str],
}

#[derive(Debug, Serialize)]
pub struct OpenApiRequestBody {
    pub required: bool,
    pub content:  RoutineRequestContent,
}

#[derive(Debug, Serialize)]
pub struct RoutineRequestContent {
    #[serde(rename = "application/json")]
    pub json:   OpenApiMediaType,
    #[serde(rename = "application/x-kalam-domain")]
    pub domain: OpenApiMediaType,
}

#[derive(Debug, Serialize)]
pub struct OpenApiMediaType {
    pub schema: OpenApiSchema,
}

#[derive(Debug, Serialize)]
pub struct RoutineResponses {
    #[serde(rename = "200")]
    pub ok:              OpenApiDescription,
    #[serde(rename = "400")]
    pub bad_request:     OpenApiReference,
    #[serde(rename = "401")]
    pub unauthorized:    OpenApiReference,
    #[serde(rename = "403")]
    pub forbidden:       OpenApiReference,
    #[serde(rename = "404")]
    pub not_found:       OpenApiReference,
    #[serde(rename = "409")]
    pub conflict:        OpenApiReference,
    #[serde(rename = "429")]
    pub too_many:        OpenApiReference,
    #[serde(rename = "500")]
    pub internal:        OpenApiReference,
    #[serde(rename = "504")]
    pub gateway_timeout: OpenApiReference,
}

impl RoutineResponses {
    fn standard() -> Self {
        let error = OpenApiReference {
            reference: STRUCTURED_ERROR_REF,
        };
        Self {
            ok:              OpenApiDescription {
                description: "Validated routine result",
            },
            bad_request:     error.clone(),
            unauthorized:    error.clone(),
            forbidden:       error.clone(),
            not_found:       error.clone(),
            conflict:        error.clone(),
            too_many:        error.clone(),
            internal:        error.clone(),
            gateway_timeout: error,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenApiReference {
    #[serde(rename = "$ref")]
    pub reference: &'static str,
}

#[derive(Debug, Serialize)]
pub struct OpenApiDescription {
    pub description: &'static str,
}

#[derive(Debug, Serialize)]
pub struct OpenApiComponents {
    #[serde(rename = "securitySchemes")]
    pub security_schemes: OpenApiSecuritySchemes,
    pub responses:        OpenApiComponentResponses,
}

impl OpenApiComponents {
    fn standard() -> Self {
        Self {
            security_schemes: OpenApiSecuritySchemes {
                bearer_auth: OpenApiHttpSecurity {
                    scheme_type:   "http",
                    scheme:        "bearer",
                    bearer_format: "JWT",
                },
            },
            responses:        OpenApiComponentResponses {
                structured_error: StructuredErrorResponse {
                    description: "Structured KalamDB error response",
                    content:     StructuredErrorContent {
                        json: OpenApiMediaType {
                            schema: structured_error_schema(),
                        },
                    },
                },
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OpenApiSecuritySchemes {
    #[serde(rename = "bearerAuth")]
    pub bearer_auth: OpenApiHttpSecurity,
}

#[derive(Debug, Serialize)]
pub struct OpenApiHttpSecurity {
    #[serde(rename = "type")]
    pub scheme_type:   &'static str,
    pub scheme:        &'static str,
    #[serde(rename = "bearerFormat")]
    pub bearer_format: &'static str,
}

#[derive(Debug, Serialize)]
pub struct OpenApiComponentResponses {
    #[serde(rename = "StructuredError")]
    pub structured_error: StructuredErrorResponse,
}

#[derive(Debug, Serialize)]
pub struct StructuredErrorResponse {
    pub description: &'static str,
    pub content:     StructuredErrorContent,
}

#[derive(Debug, Serialize)]
pub struct StructuredErrorContent {
    #[serde(rename = "application/json")]
    pub json: OpenApiMediaType,
}

/// JSON Schema object used inside the OpenAPI document.
#[derive(Debug, Clone, Serialize)]
pub struct OpenApiSchema {
    #[serde(rename = "type")]
    pub schema_type: OpenApiType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format:      Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items:       Option<Box<OpenApiSchema>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nullable:    Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default:     Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties:  Option<BTreeMap<String, OpenApiSchema>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required:    Option<Vec<String>>,
    #[serde(rename = "const", skip_serializing_if = "Option::is_none")]
    pub const_value: Option<&'static str>,
}

impl OpenApiSchema {
    fn new(schema_type: OpenApiType) -> Self {
        Self {
            schema_type,
            format: None,
            items: None,
            nullable: None,
            default: None,
            properties: None,
            required: None,
            const_value: None,
        }
    }
}

/// OpenAPI `type` is either one name or a JSON Schema union.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum OpenApiType {
    One(&'static str),
    AnyOf(&'static [&'static str]),
}

fn schema_for_input(input: &OpenApiInput) -> OpenApiSchema {
    let mut schema = sql_schema(&input.sql_type);
    if input.is_array {
        schema = OpenApiSchema {
            items: Some(Box::new(schema)),
            ..OpenApiSchema::new(OpenApiType::One("array"))
        };
    }
    OpenApiSchema {
        nullable: Some(!input.not_null),
        default: input.default_expr.clone(),
        ..schema
    }
}

fn sql_schema(sql_type: &str) -> OpenApiSchema {
    let upper = sql_type.to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or(upper.as_str());
    match base {
        "BOOL" | "BOOLEAN" => OpenApiSchema::new(OpenApiType::One("boolean")),
        "INT" | "INTEGER" | "INT4" | "SMALLINT" | "INT2" => OpenApiSchema {
            format: Some("int32"),
            ..OpenApiSchema::new(OpenApiType::One("integer"))
        },
        "BIGINT" | "INT8" | "INT64" => OpenApiSchema {
            format: Some("int64"),
            ..OpenApiSchema::new(OpenApiType::AnyOf(INT64_TYPES))
        },
        "DECIMAL" | "NUMERIC" => OpenApiSchema {
            format: Some("decimal"),
            ..OpenApiSchema::new(OpenApiType::One("string"))
        },
        "FLOAT" | "FLOAT4" | "REAL" | "DOUBLE" | "FLOAT8" => {
            OpenApiSchema::new(OpenApiType::One("number"))
        },
        "JSON" | "JSONB" => OpenApiSchema::new(OpenApiType::AnyOf(JSON_VALUE_TYPES)),
        _ => OpenApiSchema::new(OpenApiType::One("string")),
    }
}

fn object_schema(
    properties: BTreeMap<String, OpenApiSchema>,
    required: Vec<String>,
) -> OpenApiSchema {
    OpenApiSchema {
        properties: Some(properties),
        required: Some(required),
        ..OpenApiSchema::new(OpenApiType::One("object"))
    }
}

fn binary_schema() -> OpenApiSchema {
    OpenApiSchema {
        format: Some("binary"),
        ..OpenApiSchema::new(OpenApiType::One("string"))
    }
}

fn structured_error_schema() -> OpenApiSchema {
    let mut status = OpenApiSchema::new(OpenApiType::One("string"));
    status.const_value = Some("error");
    let mut properties = BTreeMap::new();
    properties.insert("status".to_string(), status);
    properties.insert("code".to_string(), OpenApiSchema::new(OpenApiType::One("string")));
    properties.insert("message".to_string(), OpenApiSchema::new(OpenApiType::One("string")));
    properties.insert("fieldPath".to_string(), OpenApiSchema::new(OpenApiType::One("string")));
    OpenApiSchema {
        properties: Some(properties),
        required: Some(vec![
            "status".to_string(),
            "code".to_string(),
            "message".to_string(),
        ]),
        ..OpenApiSchema::new(OpenApiType::One("object"))
    }
}

/// Authorized current-epoch OpenAPI document (`GET /v1/api/openapi.json`).
pub async fn openapi_v1(
    extractor: AuthSessionExtractor,
    http_req: HttpRequest,
    app_context: web::Data<Arc<AppContext>>,
) -> HttpResponse {
    let _session: AuthSession = extractor.into();
    if let Some(err_resp) = check_epoch_header(&http_req) {
        return err_resp;
    }
    let stores = app_context.system_tables().catalog_stores();
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
    let mut specs = Vec::with_capacity(routines.len());
    for routine in &routines {
        let parameters = match stores.list_parameters(&routine.routine_id) {
            Ok(parameters) => parameters,
            Err(err) => {
                return function_error_response(
                    actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_RUNTIME_ERROR",
                    err,
                );
            },
        };
        let inputs = parameters
            .into_iter()
            .filter(|parameter| parameter.mode.is_input())
            .map(|parameter| OpenApiInput {
                name:         parameter.name,
                sql_type:     parameter.type_name,
                is_array:     parameter.is_array,
                not_null:     parameter.not_null,
                default_expr: parameter.default_expr,
            })
            .collect();
        specs.push(OpenApiRoutineSpec {
            routine_id: routine.routine_id.as_str().to_string(),
            namespace: routine.namespace_id.as_str().to_string(),
            name: routine.name.clone(),
            summary: routine
                .comment
                .clone()
                .unwrap_or_else(|| routine.routine_id.as_str().to_string()),
            inputs,
        });
    }
    HttpResponse::Ok().json(OpenApiDocument::from_routines(&specs))
}

#[cfg(test)]
mod tests {
    use kalamdb_serialization::DOMAIN_EPOCH;
    use serde_json::json;

    use super::{OpenApiDocument, OpenApiInput, OpenApiRoutineSpec};

    #[test]
    fn routine_document_serializes_typed_schemas() {
        let spec = OpenApiRoutineSpec {
            routine_id: "chat.send".to_string(),
            namespace:  "chat".to_string(),
            name:       "send".to_string(),
            summary:    "Send a message".to_string(),
            inputs:     vec![
                OpenApiInput {
                    name:         "body".to_string(),
                    sql_type:     "JSON".to_string(),
                    is_array:     false,
                    not_null:     true,
                    default_expr: None,
                },
                OpenApiInput {
                    name:         "tags".to_string(),
                    sql_type:     "TEXT".to_string(),
                    is_array:     true,
                    not_null:     false,
                    default_expr: Some("'[]'".to_string()),
                },
                OpenApiInput {
                    name:         "amount".to_string(),
                    sql_type:     "NUMERIC(12, 2)".to_string(),
                    is_array:     false,
                    not_null:     true,
                    default_expr: Some("0.00".to_string()),
                },
            ],
        };
        let value = serde_json::to_value(OpenApiDocument::from_routines(&[spec])).unwrap();
        assert_eq!(value["openapi"], "3.1.0");
        assert_eq!(value["info"]["version"], format!("epoch-{DOMAIN_EPOCH}"));

        let post = &value["paths"]["/v1/functions/chat/send"]["post"];
        assert_eq!(post["operationId"], "chat.send");
        assert_eq!(post["summary"], "Send a message");
        assert_eq!(post["security"][0]["bearerAuth"], json!([]));
        assert_eq!(post["requestBody"]["required"], true);

        let schema = &post["requestBody"]["content"]["application/json"]["schema"];
        assert_eq!(schema["required"], json!(["body"]));
        assert_eq!(
            schema["properties"]["body"]["type"],
            json!(["object", "array", "string", "number", "boolean", "null"])
        );
        assert_eq!(schema["properties"]["body"]["nullable"], false);
        assert_eq!(schema["properties"]["tags"]["type"], "array");
        assert_eq!(schema["properties"]["tags"]["items"]["type"], "string");
        assert!(schema["properties"]["tags"]["items"].get("nullable").is_none());
        assert_eq!(schema["properties"]["tags"]["nullable"], true);
        assert_eq!(schema["properties"]["tags"]["default"], "'[]'");
        assert_eq!(schema["properties"]["amount"]["type"], "string");
        assert_eq!(schema["properties"]["amount"]["format"], "decimal");
        assert_eq!(
            post["requestBody"]["content"]["application/x-kalam-domain"]["schema"]["format"],
            "binary"
        );
        assert_eq!(post["responses"]["200"]["description"], "Validated routine result");
        assert_eq!(post["responses"]["409"]["$ref"], "#/components/responses/StructuredError");
        assert_eq!(value["components"]["securitySchemes"]["bearerAuth"]["scheme"], "bearer");
        let error = &value["components"]["responses"]["StructuredError"];
        assert_eq!(
            error["content"]["application/json"]["schema"]["properties"]["status"]["const"],
            "error"
        );
        assert_eq!(
            error["content"]["application/json"]["schema"]["required"],
            json!(["status", "code", "message"])
        );
    }
}
