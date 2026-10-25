//! Frontend `createKalam` client. Generated routines call `invokeRoutine`.

use kalamdb_sql::contracts::ContractSnapshot;
use serde::Serialize;

use super::text::jsdoc;
use crate::{
    error::Result,
    workflow::{
        project::templates::{escape_double_quoted_string, render_schema_gen_file},
        schema::{
            naming::AssignedNames,
            output::GeneratedHeader,
            procedures::{ProcedureCatalog, ProcedureSpec},
        },
    },
};

const CLIENT_TEMPLATE: &str = "src/generated/kalam.ts";

#[derive(Serialize)]
struct ClientContext<'a> {
    #[serde(flatten)]
    file: GeneratedHeader,
    has_type_imports: bool,
    has_calls: bool,
    type_imports: String,
    namespaces: Vec<NamespaceContext<'a>>,
}

#[derive(Serialize)]
struct NamespaceContext<'a> {
    object_ident: &'a str,
    procedures: Vec<ProcedureContext>,
}

#[derive(Serialize)]
struct ProcedureContext {
    jsdoc: String,
    routine_id: String,
    signature: String,
    schema: String,
    name: String,
    method_ident: String,
    has_params: bool,
    request_ident: String,
    result_ident: String,
    cardinality: String,
    params: String,
}

pub fn generate_client_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
) -> Result<String> {
    let procedures = ProcedureCatalog::from_snapshot(snapshot, names);
    render_client_source(hash, &procedures)
}

pub(super) fn render_client_source(
    hash: &str,
    procedures: &ProcedureCatalog<'_>,
) -> Result<String> {
    let type_imports = procedures.schema_type_imports(false);
    let namespaces: Vec<NamespaceContext<'_>> = procedures
        .namespaces()
        .iter()
        .map(|namespace| NamespaceContext {
            object_ident: namespace.object_ident.as_str(),
            procedures: namespace.procedures.iter().map(procedure_context).collect(),
        })
        .collect();
    let has_calls = namespaces.iter().any(|namespace| !namespace.procedures.is_empty());
    render_schema_gen_file(
        "typescript",
        CLIENT_TEMPLATE,
        &ClientContext {
            file: GeneratedHeader::from_hash(hash),
            has_type_imports: !type_imports.is_empty(),
            has_calls,
            type_imports,
            namespaces,
        },
    )
}

fn procedure_context(spec: &ProcedureSpec<'_>) -> ProcedureContext {
    ProcedureContext {
        jsdoc: jsdoc(spec.comment(), "      "),
        routine_id: escape_double_quoted_string(spec.routine.routine_id.as_str()),
        signature: escape_double_quoted_string(&spec.routine.signature),
        schema: escape_double_quoted_string(&spec.routine.schema),
        name: escape_double_quoted_string(&spec.routine.name),
        method_ident: spec.method_ident.clone(),
        has_params: spec.has_params(),
        request_ident: spec.request_ident(),
        result_ident: spec.result_ident(),
        cardinality: spec.routine.cardinality.as_str().to_string(),
        params: param_slots(spec),
    }
}

fn param_slots(spec: &ProcedureSpec<'_>) -> String {
    spec.parameters()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let json = matches!(field.data_type, Some(kalamdb_commons::KalamDataType::Json));
            format!(
                "{{name:\"{}\",slot:{},json:{}}}",
                escape_double_quoted_string(&field.name),
                index + 1,
                if json { "true" } else { "false" }
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
