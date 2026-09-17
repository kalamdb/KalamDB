//! Dart row codecs shared by tables and procedure request types.

use std::collections::HashSet;

use kalamdb_sql::contracts::{ContractField, ContractSnapshot, ContractTypeKind};
use serde::Serialize;

pub use crate::workflow::project::templates::escape_js_single_quoted_string as escape_dart_string;
use crate::workflow::schema::{
    naming::{camel_case, AssignedNames},
    output::line_docs,
    types::{render_field_type, TargetLang},
};

pub struct DartColumn {
    pub sql_name:   String,
    pub field_name: String,
    pub dart_type:  String,
    pub base_type:  String,
    pub nullable:   bool,
    pub is_key:     bool,
    pub named_type: Option<String>,
    pub is_enum:    bool,
}

impl DartColumn {
    pub fn from_field(
        field: &ContractField,
        names: &AssignedNames,
        snapshot: &ContractSnapshot,
    ) -> Self {
        let dart_type = render_field_type(field, names, TargetLang::Dart);
        let is_enum = field
            .type_id
            .as_ref()
            .and_then(|id| snapshot.types.get(id.as_str()))
            .is_some_and(|ty| matches!(ty.kind, ContractTypeKind::Enum { .. }));
        let base_type = if let Some(type_id) = &field.type_id {
            names.type_ident(type_id.as_str()).to_string()
        } else {
            crate::workflow::schema::types::builtin(&field.type_name, TargetLang::Dart).to_string()
        };
        Self {
            sql_name: field.name.clone(),
            field_name: dart_field_name(&field.name),
            dart_type,
            named_type: field.type_id.as_ref().map(|id| names.type_ident(id.as_str()).to_string()),
            is_enum,
            nullable: !field.not_null,
            is_key: field.name.eq_ignore_ascii_case("id"),
            base_type,
        }
    }

    pub fn encode(&self) -> String {
        let name = &self.field_name;
        if self.named_type.is_some() {
            if self.is_enum {
                return if self.nullable {
                    format!("{name}?.wire")
                } else {
                    format!("{name}.wire")
                };
            }
            return if self.nullable {
                format!("{name}?.toJson()")
            } else {
                format!("{name}.toJson()")
            };
        }
        match (self.base_type.as_str(), self.nullable) {
            ("DateTime", true) => format!("{name}?.toIso8601String()"),
            ("DateTime", false) => format!("{name}.toIso8601String()"),
            _ => name.to_string(),
        }
    }

    pub fn decode(&self) -> String {
        self.decode_expr(&format!("json['{}']", escape_dart_string(&self.sql_name)))
    }

    pub fn decode_expr(&self, json: &str) -> String {
        if let Some(named) = &self.named_type {
            if self.is_enum {
                return if self.nullable {
                    format!(
                        "{json} == null ? null : {named}.values.firstWhere((value) => value.wire \
                         == _asString({json}))"
                    )
                } else {
                    format!("{named}.values.firstWhere((value) => value.wire == _asString({json}))")
                };
            }
            return if self.nullable {
                format!("{json} == null ? null : {named}.fromJson(_asJsonMap({json}))")
            } else {
                format!("{named}.fromJson(_asJsonMap({json}))")
            };
        }
        match (self.base_type.as_str(), self.nullable) {
            ("int", false) => format!("_asInt({json})"),
            ("int", true) => format!("_asIntOrNull({json})"),
            ("double", false) => format!("_asDouble({json})"),
            ("double", true) => format!("_asDoubleOrNull({json})"),
            ("bool", false) => format!("_asBool({json})"),
            ("bool", true) => format!("_asBoolOrNull({json})"),
            ("DateTime", false) => format!("_asDateTime({json})"),
            ("DateTime", true) => format!("_asDateTimeOrNull({json})"),
            ("Map<String, Object?>", false) => format!("_asJsonMap({json})"),
            ("Map<String, Object?>", true) => format!("_asJsonMapOrNull({json})"),
            ("List<double>", false) => format!("_asDoubleList({json})"),
            ("List<double>", true) => format!("_asDoubleListOrNull({json})"),
            (_, false) => format!("_asString({json})"),
            (_, true) => format!("_asStringOrNull({json})"),
        }
    }
}

#[derive(Serialize)]
pub struct RowClassContext {
    pub doc:           String,
    pub class_name:    String,
    pub fields:        Vec<RowFieldContext>,
    pub with_row_key:  bool,
    pub kalam_row_key: String,
}

#[derive(Serialize)]
pub struct RowFieldContext {
    pub sql_name:   String,
    pub field_name: String,
    pub dart_type:  String,
    pub required:   bool,
    pub encode:     String,
    pub decode:     String,
}

pub fn row_class_context(
    class_name: &str,
    fields: &[ContractField],
    names: &AssignedNames,
    snapshot: &ContractSnapshot,
    comment: Option<&str>,
    with_row_key: bool,
) -> RowClassContext {
    let columns: Vec<DartColumn> = fields
        .iter()
        .map(|field| DartColumn::from_field(field, names, snapshot))
        .collect();
    RowClassContext {
        doc: line_docs(comment, "/// "),
        class_name: class_name.to_string(),
        fields: columns
            .iter()
            .map(|column| RowFieldContext {
                sql_name:   escape_dart_string(&column.sql_name),
                field_name: column.field_name.clone(),
                dart_type:  column.dart_type.clone(),
                required:   !column.nullable,
                encode:     column.encode(),
                decode:     column.decode(),
            })
            .collect(),
        with_row_key,
        kalam_row_key: kalam_row_key_expr(&columns),
    }
}

fn kalam_row_key_expr(columns: &[DartColumn]) -> String {
    match columns.iter().find(|column| column.is_key).or_else(|| columns.first()) {
        Some(key) if key.base_type == "String" && !key.nullable => key.field_name.clone(),
        Some(key) => format!("{}.toString()", key.field_name),
        None => "''".to_string(),
    }
}

fn dart_field_name(sql_name: &str) -> String {
    sanitize_ident(&camel_case(sql_name))
}

pub fn unique_ident(base: String, used: &mut HashSet<String>) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let mut index = 2u32;
    loop {
        let candidate = format!("{base}{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn sanitize_ident(value: &str) -> String {
    let mut ident: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if ident.is_empty() || ident.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        ident.insert(0, 'n');
    }
    if DART_KEYWORDS.contains(&ident.as_str()) {
        ident.push('_');
    }
    ident
}

const DART_KEYWORDS: &[&str] = &[
    "assert", "break", "case", "catch", "class", "const", "continue", "default", "do", "else",
    "enum", "extends", "false", "final", "finally", "for", "if", "in", "is", "new", "null",
    "rethrow", "return", "super", "switch", "this", "throw", "true", "try", "var", "void", "while",
    "with", "yield",
];
