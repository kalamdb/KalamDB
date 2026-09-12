//! TypeScript schema types and Drizzle `kTable` bindings.

use kalamdb_sql::contracts::{
    table_payload_tag, ContractField, ContractSnapshot, ContractTableKind, ContractTypeKind,
};
use serde::Serialize;

use super::text::jsdoc;
use crate::{
    error::Result,
    workflow::{
        project::templates::{escape_double_quoted_string, render_schema_gen_file},
        schema::{
            naming::{value_ident, AssignedNames},
            output::{canonical_row_type_id, row_type_ident, type_comment, GeneratedHeader},
            procedures::{ProcedureCatalog, ProcedureSpec},
            types::{render_field_type, TargetLang},
        },
    },
};

const SCHEMA_TEMPLATE: &str = "src/generated/schema.ts";

#[derive(Serialize)]
struct SchemaContext {
    #[serde(flatten)]
    file:      GeneratedHeader,
    needs_orm: bool,
    decls:     Vec<SchemaDecl>,
    tables:    Vec<TableDecl>,
}

#[derive(Serialize, Default)]
struct SchemaDecl {
    jsdoc:            String,
    ident:            String,
    is_enum:          bool,
    union:            String,
    is_object:        bool,
    fields:           Vec<FieldDecl>,
    is_alias:         bool,
    alias_of:         String,
    is_topic:         bool,
    is_topic_empty:   bool,
    variants:         Vec<TopicVariant>,
    is_request_empty: bool,
    is_result:        bool,
    ty:               String,
}

#[derive(Serialize)]
struct FieldDecl {
    name: String,
    ty:   String,
}

#[derive(Serialize)]
struct TopicVariant {
    tag:        String,
    row_ident:  String,
    terminator: String,
}

#[derive(Serialize)]
struct TableDecl {
    var_name:   String,
    factory:    &'static str,
    table_name: String,
    columns:    Vec<ColumnDecl>,
}

#[derive(Serialize)]
struct ColumnDecl {
    name: String,
    expr: String,
}

pub fn generate_schema_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
) -> Result<String> {
    let procedures = ProcedureCatalog::from_snapshot(snapshot, names);
    render_schema_source(snapshot, hash, names, &procedures)
}

pub(super) fn render_schema_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> Result<String> {
    render_schema_gen_file(
        "typescript",
        SCHEMA_TEMPLATE,
        &SchemaContext {
            file:      GeneratedHeader::from_hash(hash),
            needs_orm: !snapshot.tables.is_empty(),
            decls:     schema_decls(snapshot, names, procedures),
            tables:    table_decls(snapshot),
        },
    )
}

fn schema_decls(
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> Vec<SchemaDecl> {
    let mut decls = Vec::new();
    for (id, ty) in &snapshot.types {
        match &ty.kind {
            ContractTypeKind::Enum { labels } => {
                decls.push(SchemaDecl {
                    jsdoc: jsdoc(ty.comment.as_deref(), ""),
                    ident: names.type_ident(id).to_string(),
                    is_enum: true,
                    union: labels
                        .iter()
                        .map(|label| format!("\"{}\"", escape_double_quoted_string(label)))
                        .collect::<Vec<_>>()
                        .join(" | "),
                    ..SchemaDecl::default()
                });
            },
            ContractTypeKind::Composite { fields } => {
                decls.push(object_decl(names.type_ident(id), fields, names, ty.comment.as_deref()));
            },
            ContractTypeKind::TopicPayload { .. }
            | ContractTypeKind::ImplicitTableRow { .. }
            | ContractTypeKind::RowAlias { .. } => {},
        }
    }

    for table in snapshot.tables.values() {
        let canonical_id = canonical_row_type_id(table).to_string();
        let comment = type_comment(snapshot, &canonical_id)
            .or_else(|| type_comment(snapshot, table.row_type_id.as_str()));
        decls.push(object_decl(names.type_ident(&canonical_id), &table.fields, names, comment));
        if let Some(alias) = &table.row_alias_id {
            let alias_ident = names.type_ident(alias.as_str());
            let row_ident = names.type_ident(table.row_type_id.as_str());
            if alias_ident != row_ident {
                decls.push(SchemaDecl {
                    ident: row_ident.to_string(),
                    is_alias: true,
                    alias_of: alias_ident.to_string(),
                    ..SchemaDecl::default()
                });
            }
        }
    }

    for (id, ty) in &snapshot.types {
        if let ContractTypeKind::TopicPayload { sources, .. } = &ty.kind {
            decls.push(topic_decl(
                names.type_ident(id),
                sources,
                snapshot,
                names,
                ty.comment.as_deref(),
            ));
        }
    }

    for spec in procedures.procedures() {
        decls.extend(procedure_decls(spec, names));
    }
    decls
}

fn object_decl(
    ident: &str,
    fields: &[ContractField],
    names: &AssignedNames,
    comment: Option<&str>,
) -> SchemaDecl {
    SchemaDecl {
        jsdoc: jsdoc(comment, ""),
        ident: ident.to_string(),
        is_object: true,
        fields: field_decls(fields, names),
        ..SchemaDecl::default()
    }
}

fn topic_decl(
    ident: &str,
    sources: &[String],
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    comment: Option<&str>,
) -> SchemaDecl {
    if sources.is_empty() {
        return SchemaDecl {
            jsdoc: jsdoc(comment, ""),
            ident: ident.to_string(),
            is_topic_empty: true,
            ..SchemaDecl::default()
        };
    }
    let last = sources.len() - 1;
    SchemaDecl {
        jsdoc: jsdoc(comment, ""),
        ident: ident.to_string(),
        is_topic: true,
        variants: sources
            .iter()
            .enumerate()
            .map(|(index, table_id)| TopicVariant {
                tag:        escape_double_quoted_string(&table_payload_tag(table_id)),
                row_ident:  row_type_ident(snapshot, names, table_id).to_string(),
                terminator: if index == last {
                    ";".to_string()
                } else {
                    String::new()
                },
            })
            .collect(),
        ..SchemaDecl::default()
    }
}

fn procedure_decls(spec: &ProcedureSpec<'_>, names: &AssignedNames) -> Vec<SchemaDecl> {
    let mut decls = Vec::new();
    let request = spec.request_ident();
    if spec.has_params() {
        decls.push(object_decl(&request, spec.parameters(), names, spec.comment()));
    } else {
        decls.push(SchemaDecl {
            jsdoc: jsdoc(spec.comment(), ""),
            ident: request,
            is_request_empty: true,
            ..SchemaDecl::default()
        });
    }
    decls.push(SchemaDecl {
        jsdoc: jsdoc(spec.comment(), ""),
        ident: spec.result_ident(),
        is_result: true,
        ty: spec
            .return_type()
            .map(|ret| render_field_type(ret, names, TargetLang::TypeScript))
            .unwrap_or_else(|| "void".to_string()),
        ..SchemaDecl::default()
    });
    decls
}

fn field_decls(fields: &[ContractField], names: &AssignedNames) -> Vec<FieldDecl> {
    fields
        .iter()
        .map(|field| FieldDecl {
            name: field.name.clone(),
            ty:   render_field_type(field, names, TargetLang::TypeScript),
        })
        .collect()
}

fn table_decls(snapshot: &ContractSnapshot) -> Vec<TableDecl> {
    snapshot
        .tables
        .values()
        .map(|table| TableDecl {
            var_name:   value_ident(&table.schema, &table.name, false),
            factory:    match table.kind {
                ContractTableKind::User => "kTable.user",
                ContractTableKind::Shared => "kTable.shared",
                ContractTableKind::Stream => "kTable.stream",
                ContractTableKind::Unspecified => "kTable",
            },
            table_name: escape_double_quoted_string(&table.table_id),
            columns:    table
                .fields
                .iter()
                .map(|field| ColumnDecl {
                    name: field.name.clone(),
                    expr: drizzle_column(field),
                })
                .collect(),
        })
        .collect()
}

fn drizzle_column(field: &ContractField) -> String {
    let name = escape_double_quoted_string(&field.name);
    let mut expr = if field.type_id.is_some() {
        format!("jsonb(\"{name}\")")
    } else {
        match field.type_name.to_ascii_uppercase().as_str() {
            "BOOLEAN" | "BOOL" => format!("boolean(\"{name}\")"),
            "INT" | "INTEGER" | "INT4" | "SERIAL" | "SMALLINT" | "INT2" => {
                format!("integer(\"{name}\")")
            },
            "BIGINT" | "INT8" | "INT64" | "BIGSERIAL" => {
                format!("bigint(\"{name}\", {{ mode: \"bigint\" }})")
            },
            "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME" => {
                format!("timestamp(\"{name}\", {{ mode: \"date\" }})")
            },
            "JSON" | "JSONB" | "FILE" => format!("jsonb(\"{name}\")"),
            _ => format!("text(\"{name}\")"),
        }
    };
    if field.name == "id" {
        expr.push_str(".primaryKey()");
    } else if field.not_null {
        expr.push_str(".notNull()");
    }
    expr
}
