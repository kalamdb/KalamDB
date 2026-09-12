//! Typed Dart procedure clients generated from [`ProcedureCatalog`].

use kalamdb_sql::contracts::ContractSnapshot;
use serde::Serialize;

use super::rows::{escape_dart_string, row_class_context, DartColumn, RowClassContext};
use crate::workflow::schema::{
    naming::{pascal_case, AssignedNames},
    output::line_docs,
    procedures::{ProcedureCatalog, ProcedureSpec},
    types::{render_field_type, TargetLang},
};

#[derive(Serialize)]
pub struct FunctionsContext {
    pub namespaces: Vec<NamespaceContext>,
}

#[derive(Serialize)]
pub struct NamespaceContext {
    pub object_ident: String,
    pub class_name:   String,
    pub procedures:   Vec<MethodContext>,
}

#[derive(Serialize)]
pub struct MethodContext {
    pub doc:           String,
    pub result_ident:  String,
    pub method_ident:  String,
    pub has_params:    bool,
    pub request_ident: String,
    pub call_sql:      String,
    pub params_list:   String,
    pub return_block:  String,
}

#[derive(Serialize)]
pub struct ResultAlias {
    pub doc:   String,
    pub ident: String,
    pub ty:    String,
}

pub fn procedure_types(
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> (Vec<RowClassContext>, Vec<ResultAlias>) {
    let mut classes = Vec::new();
    let mut aliases = Vec::new();
    for spec in procedures.procedures() {
        if spec.has_params() {
            classes.push(row_class_context(
                &spec.request_ident(),
                spec.parameters(),
                names,
                snapshot,
                spec.comment(),
                false,
            ));
        }
        aliases.push(ResultAlias {
            doc:   line_docs(spec.comment(), "/// "),
            ident: spec.result_ident(),
            ty:    spec
                .return_type()
                .map(|ret| render_field_type(ret, names, TargetLang::Dart))
                .unwrap_or_else(|| "void".to_string()),
        });
    }
    (classes, aliases)
}

pub fn functions_context(
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> FunctionsContext {
    FunctionsContext {
        namespaces: procedures
            .namespaces()
            .iter()
            .map(|namespace| NamespaceContext {
                object_ident: namespace.object_ident.clone(),
                class_name:   namespace_client_class(namespace.schema),
                procedures:   namespace
                    .procedures
                    .iter()
                    .map(|spec| method_context(spec, names, snapshot))
                    .collect(),
            })
            .collect(),
    }
}

fn namespace_client_class(schema: &str) -> String {
    format!("{}Functions", pascal_case(schema))
}

fn method_context(
    spec: &ProcedureSpec<'_>,
    names: &AssignedNames,
    snapshot: &ContractSnapshot,
) -> MethodContext {
    let params_list = spec
        .parameters()
        .iter()
        .map(|field| format!("input.{}", DartColumn::from_field(field, names, snapshot).encode()))
        .collect::<Vec<_>>()
        .join(", ");
    let return_block = match spec.return_type() {
        None => "    _kalamCallResult(response);\n".to_string(),
        Some(field) => {
            let decode = DartColumn::from_field(field, names, snapshot).decode_expr("result");
            format!("    final result = _kalamCallResult(response);\n    return {decode};\n")
        },
    };
    MethodContext {
        doc: line_docs(spec.comment(), "/// "),
        result_ident: spec.result_ident(),
        method_ident: spec.method_ident.clone(),
        has_params: spec.has_params(),
        request_ident: spec.request_ident(),
        call_sql: escape_dart_string(&spec.call_sql),
        params_list,
        return_block,
    }
}
