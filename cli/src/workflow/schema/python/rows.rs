//! Python row dataclasses and the column metadata generated tables declare.

use std::collections::HashSet;

use kalamdb_sql::contracts::{ContractField, ContractSnapshot, ContractTypeKind};

pub use crate::workflow::project::templates::escape_double_quoted_string as escape_python_string;
use crate::workflow::schema::{
    naming::{snake_case, AssignedNames},
    output::unique_ident,
    types::{builtin, is_file_field, render_field_type, TargetLang},
};

/// One column, resolved far enough to emit its annotation, codecs, and
/// declaration. Mirrors `DartColumn` so the enum/composite/domain/FILE matrix
/// stays in one shape across targets.
#[derive(Clone)]
pub struct PyColumn {
    pub sql_name: String,
    pub field_name: String,
    pub py_type: String,
    pub base_type: String,
    pub sql_type: String,
    pub nullable: bool,
    pub is_array: bool,
    pub is_key: bool,
    pub pk_ordinal: u16,
    pub has_default: bool,
    pub is_read_only: bool,
    pub named_type: Option<String>,
    pub is_enum: bool,
}

impl PyColumn {
    pub fn from_field(
        field: &ContractField,
        names: &AssignedNames,
        snapshot: &ContractSnapshot,
    ) -> Self {
        let py_type = render_field_type(field, names, TargetLang::Python);
        let is_file = is_file_field(field);
        let resolved = field.type_id.as_ref().and_then(|id| snapshot.types.get(id.as_str()));
        let is_enum = resolved.is_some_and(|ty| matches!(ty.kind, ContractTypeKind::Enum { .. }));
        let is_domain =
            resolved.is_some_and(|ty| matches!(ty.kind, ContractTypeKind::Domain { .. }));

        let base_type = if let Some(ContractTypeKind::Domain { base_type, .. }) =
            resolved.map(|ty| &ty.kind)
        {
            builtin(&base_type.type_name, TargetLang::Python).to_string()
        } else if is_file {
            "FileRef".to_string()
        } else if let Some(type_id) = &field.type_id {
            names.type_ident(type_id.as_str()).to_string()
        } else {
            builtin(&field.type_name, TargetLang::Python).to_string()
        };

        // A domain decodes as its base scalar and FILE has its own codec, so
        // neither carries a named type through to the row class.
        let named_type = if is_domain || is_file {
            None
        } else {
            field.type_id.as_ref().map(|id| names.type_ident(id.as_str()).to_string())
        };

        Self {
            sql_name: field.name.clone(),
            field_name: python_field_name(&field.name),
            py_type,
            base_type,
            sql_type: sql_type_label(field),
            nullable: !field.not_null,
            is_array: field.is_array,
            is_key: field.primary_key,
            pk_ordinal: field.primary_key_ordinal.unwrap_or(0),
            has_default: field.has_default,
            is_read_only: field.is_read_only || field.is_generated || field.is_identity,
            named_type,
            is_enum,
        }
    }

    /// `as_int(row.get("id"))`
    pub fn decode(&self) -> String {
        self.decode_expr(&format!("row.get(\"{}\")", escape_python_string(&self.sql_name)))
    }

    fn decode_expr(&self, value: &str) -> String {
        if self.is_array {
            let mut element = self.clone();
            element.is_array = false;
            element.nullable = false;
            let item = element.decode_callable();
            return if self.nullable {
                format!("as_list_or_none({value}, {item})")
            } else {
                format!("as_list({value}, {item})")
            };
        }
        if let Some(named) = &self.named_type {
            let helper = if self.is_enum { "as_enum" } else { "as_struct" };
            let suffix = if self.nullable { "_or_none" } else { "" };
            return format!("{helper}{suffix}({value}, {named})");
        }
        match (self.base_type.as_str(), self.nullable) {
            ("Any", _) => format!("as_json({value})"),
            ("FileRef", false) => format!("as_file_ref({value})"),
            ("FileRef", true) => format!("as_file_ref_or_none({value})"),
            ("List[float]", false) => format!("as_float_list({value})"),
            ("List[float]", true) => format!("as_list_or_none({value}, as_float)"),
            (scalar, nullable) => {
                let helper = scalar_decoder(scalar);
                let suffix = if nullable { "_or_none" } else { "" };
                format!("{helper}{suffix}({value})")
            },
        }
    }

    /// The per-element callable passed to `as_list`.
    fn decode_callable(&self) -> String {
        if let Some(named) = &self.named_type {
            let helper = if self.is_enum { "as_enum" } else { "as_struct" };
            return format!("lambda item: {helper}(item, {named})");
        }
        match self.base_type.as_str() {
            "Any" => "as_json".to_string(),
            "FileRef" => "as_file_ref".to_string(),
            "List[float]" => "as_float_list".to_string(),
            scalar => scalar_decoder(scalar).to_string(),
        }
    }

    /// `self.status.value`-equivalent, written through the None-safe encoders.
    pub fn encode(&self) -> String {
        let field = format!("self.{}", self.field_name);
        if self.is_array {
            let mut element = self.clone();
            element.is_array = false;
            return match element.encode_callable() {
                Some(item) => format!("encode_list({field}, {item})"),
                None => field,
            };
        }
        match self.encode_callable() {
            Some(encoder) => format!("{encoder}({field})"),
            None => field,
        }
    }

    /// The encoder for one value, or `None` when the value goes out as-is.
    fn encode_callable(&self) -> Option<String> {
        if self.named_type.is_some() {
            return Some(if self.is_enum { "encode_enum" } else { "encode_row" }.to_string());
        }
        match self.base_type.as_str() {
            "FileRef" => Some("encode_file".to_string()),
            "dt.datetime" => Some("encode_datetime".to_string()),
            "dt.date" => Some("encode_date".to_string()),
            "dt.time" => Some("encode_time".to_string()),
            _ => None,
        }
    }

    /// The column's value type, i.e. its annotation with `Optional` stripped.
    /// Generated columns classes declare `Column[T]` so `mypy --strict` can see
    /// the type even where a `Column(...)` fallback gives it nothing to infer.
    pub fn value_type(&self) -> String {
        self.py_type
            .strip_prefix("Optional[")
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or(&self.py_type)
            .to_string()
    }

    /// `_kalam.bigint("id").primary_key().server_default()`
    pub fn column_expr(&self, composite_key: bool) -> String {
        let name = escape_python_string(&self.sql_name);
        // Factories are called through the module alias because these
        // expressions are evaluated in a class body: a column named `text`
        // binds that name before the next line looks the factory up, and the
        // class attribute would win.
        let mut expr = match self.factory() {
            Some(factory) if factory == "embedding" => format!(
                "{SDK_ALIAS}.embedding(\"{name}\", {})",
                embedding_dimensions(&self.sql_type)
            ),
            Some(factory) => format!("{SDK_ALIAS}.{factory}(\"{name}\")"),
            // Enums, composites, domains and anything else keep their SQL type
            // verbatim; nothing in the runtime needs to interpret it.
            None => format!(
                "{SDK_ALIAS}.Column(\"{name}\", \"{}\")",
                escape_python_string(&self.sql_type)
            ),
        };
        if self.is_key {
            if composite_key {
                expr.push_str(&format!(".primary_key({})", self.pk_ordinal));
            } else {
                expr.push_str(".primary_key()");
            }
        }
        if !self.nullable {
            expr.push_str(".not_null()");
        }
        if self.has_default {
            expr.push_str(".server_default()");
        }
        if self.is_read_only {
            expr.push_str(".read_only()");
        }
        expr
    }

    /// Factory name for a plain scalar, or `None` to fall back to `Column(...)`.
    fn factory(&self) -> Option<&'static str> {
        if self.is_array || self.named_type.is_some() {
            return None;
        }
        let base = self.sql_type.split('(').next().unwrap_or(&self.sql_type).to_ascii_uppercase();
        Some(match base.as_str() {
            "BOOLEAN" | "BOOL" => "boolean",
            "BIGINT" | "INT8" | "INT64" | "BIGSERIAL" => "bigint",
            "INT" | "INTEGER" | "INT4" | "SERIAL" => "integer",
            "SMALLINT" | "INT2" => "smallint",
            "TEXT" => "text",
            "UUID" => "uuid",
            "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME" => "timestamp",
            "DATE" => "date",
            "TIME" => "time",
            "DOUBLE" | "FLOAT" | "FLOAT4" | "FLOAT8" | "REAL" => "double",
            "BYTES" | "BYTEA" | "BLOB" | "BINARY" => "bytes_",
            "JSON" | "JSONB" => "json",
            "FILE" => "file",
            "EMBEDDING" => "embedding",
            _ => return None,
        })
    }
}

fn scalar_decoder(py_type: &str) -> &'static str {
    match py_type {
        "int" => "as_int",
        "float" => "as_float",
        "bool" => "as_bool",
        "bytes" => "as_bytes",
        "dt.datetime" => "as_datetime",
        "dt.date" => "as_date",
        "dt.time" => "as_time",
        _ => "as_str",
    }
}

fn sql_type_label(field: &ContractField) -> String {
    let base = field
        .data_type
        .map(|data_type| data_type.sql_name())
        .unwrap_or_else(|| field.type_name.clone());
    if field.is_array {
        format!("{base}[]")
    } else {
        base
    }
}

fn embedding_dimensions(sql_type: &str) -> u32 {
    sql_type
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .and_then(|(dims, _)| dims.trim().parse().ok())
        .unwrap_or(1)
}

/// Module alias the generated file imports the runtime under. Reserved against
/// generated attribute names, so a column or table can never replace it.
pub const SDK_ALIAS: &str = "_kalam";

/// Names the generated dataclass defines as methods. A field may not take one:
/// an instance attribute would hide the method on every decoded row.
const RESERVED_ROW_ATTRS: &[&str] = &["from_row", "to_row"];

/// Give every column a distinct Python attribute name.
///
/// Two SQL names can snake_case to the same identifier (`user_id` and
/// `user__id`), which would repeat a keyword argument in `from_row` and
/// overwrite one declaration in the columns class. Allocating over the whole
/// field list keeps them apart, and the SQL name is carried separately, so no
/// column is renamed on the wire. The row and columns classes run this over the
/// same field list and so agree on the result.
pub fn assign_unique_field_names(columns: &mut [PyColumn]) {
    let mut used: HashSet<String> =
        RESERVED_ROW_ATTRS.iter().map(|name| (*name).to_string()).collect();
    for column in columns.iter_mut() {
        column.field_name = unique_ident(python_field_name(&column.sql_name), &mut used);
    }
}

/// PEP 8 field name, with a trailing underscore on keywords. The SQL name is
/// carried separately, so renaming the attribute never renames the column.
pub fn python_field_name(sql_name: &str) -> String {
    let mut ident = snake_case(sql_name);
    if PYTHON_KEYWORDS.contains(&ident.as_str()) {
        ident.push('_');
    }
    ident
}

/// Hard keywords, the three constants, and the soft keywords from 3.10/3.12 so
/// output stays valid as the floor moves.
const PYTHON_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del",
    "elif", "else", "except", "false", "finally", "for", "from", "global", "if", "import", "in",
    "is", "lambda", "match", "none", "nonlocal", "not", "or", "pass", "raise", "return", "true",
    "try", "type", "while", "with", "yield",
];
