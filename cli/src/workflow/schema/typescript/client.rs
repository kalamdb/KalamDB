//! Frontend `createKalam` CALL client generated from [`ProcedureCatalog`].

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
    file:             GeneratedHeader,
    has_type_imports: bool,
    type_imports:     String,
    namespaces:       Vec<NamespaceContext<'a>>,
}

#[derive(Serialize)]
struct NamespaceContext<'a> {
    object_ident: &'a str,
    procedures:   Vec<ProcedureContext>,
}

#[derive(Serialize)]
struct ProcedureContext {
    jsdoc:         String,
    method_ident:  String,
    has_params:    bool,
    request_ident: String,
    result_ident:  String,
    call_sql:      String,
    params_list:   String,
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
    let namespaces = procedures
        .namespaces()
        .iter()
        .map(|namespace| NamespaceContext {
            object_ident: namespace.object_ident.as_str(),
            procedures:   namespace.procedures.iter().map(procedure_context).collect(),
        })
        .collect();
    render_schema_gen_file(
        "typescript",
        CLIENT_TEMPLATE,
        &ClientContext {
            file: GeneratedHeader::from_hash(hash),
            has_type_imports: !type_imports.is_empty(),
            type_imports,
            namespaces,
        },
    )
}

fn procedure_context(spec: &ProcedureSpec<'_>) -> ProcedureContext {
    ProcedureContext {
        jsdoc:         jsdoc(spec.comment(), "      "),
        method_ident:  spec.method_ident.clone(),
        has_params:    spec.has_params(),
        request_ident: spec.request_ident(),
        result_ident:  spec.result_ident(),
        call_sql:      escape_double_quoted_string(&spec.call_sql),
        params_list:   spec
            .parameters()
            .iter()
            .map(|param| format!("input.{}", param.name))
            .collect::<Vec<_>>()
            .join(", "),
    }
}
