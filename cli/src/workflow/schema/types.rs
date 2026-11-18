//! Map contract fields onto generated TypeScript, Dart, Rust, and Python types.

use kalamdb_sql::contracts::ContractField;

use crate::workflow::schema::naming::AssignedNames;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetLang {
    TypeScript,
    Dart,
    Rust,
    Python,
}

pub fn render_field_type(field: &ContractField, names: &AssignedNames, lang: TargetLang) -> String {
    let mut inner = named_or_builtin(field, names, lang);
    if field.is_array {
        inner = match lang {
            TargetLang::TypeScript => format!("{inner}[]"),
            TargetLang::Dart => format!("List<{inner}>"),
            TargetLang::Rust => format!("Vec<{inner}>"),
            TargetLang::Python => format!("List[{inner}]"),
        };
    }
    if field.not_null {
        inner
    } else {
        match lang {
            TargetLang::TypeScript => format!("{inner} | null"),
            TargetLang::Dart => format!("{inner}?"),
            TargetLang::Rust => format!("Option<{inner}>"),
            TargetLang::Python => format!("Optional[{inner}]"),
        }
    }
}

fn named_or_builtin(field: &ContractField, names: &AssignedNames, lang: TargetLang) -> String {
    if is_file_field(field) {
        return match lang {
            // Dart already has the FILE column type in kalam_link, exported by kalam_sync.
            TargetLang::Dart => "KalamFileRef".to_string(),
            // Python re-exports the native FileRef from kalamdb.schema.
            TargetLang::Python => "FileRef".to_string(),
            TargetLang::TypeScript | TargetLang::Rust => "File".to_string(),
        };
    }
    if let Some(type_id) = &field.type_id {
        return names.type_ident(type_id.as_str()).to_string();
    }
    let sql_name = field
        .data_type
        .map(|data_type| data_type.sql_name())
        .unwrap_or_else(|| field.type_name.clone());
    builtin(&sql_name, lang).to_string()
}

pub fn is_file_field(field: &ContractField) -> bool {
    field.type_name.eq_ignore_ascii_case("FILE")
        || field.type_name.eq_ignore_ascii_case("pg_catalog.file")
        || field
            .type_id
            .as_ref()
            .is_some_and(|type_id| type_id.as_str() == "pg_catalog.file")
}

pub fn builtin(sql_type: &str, lang: TargetLang) -> &'static str {
    let normalized = sql_type.trim().trim_end_matches(',').to_ascii_uppercase();
    let base = normalized.split('(').next().unwrap_or(&normalized);
    match (lang, base) {
        (TargetLang::TypeScript, "BOOLEAN" | "BOOL") => "boolean",
        (TargetLang::TypeScript, "INT" | "INTEGER" | "INT4" | "SMALLINT" | "INT2" | "SERIAL") => {
            "number"
        },
        (TargetLang::TypeScript, "BIGINT" | "INT8" | "INT64" | "BIGSERIAL") => "bigint",
        (TargetLang::TypeScript, "DECIMAL" | "NUMERIC") => "string",
        (TargetLang::TypeScript, "BYTES" | "BYTEA" | "BLOB" | "BINARY") => "Uint8Array",
        (TargetLang::TypeScript, "JSON" | "JSONB") => "JsonValue",
        (TargetLang::TypeScript, "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME") => "Date",
        (TargetLang::TypeScript, "FILE") => "File",
        (TargetLang::TypeScript, _) => "string",

        (TargetLang::Dart, "BOOLEAN" | "BOOL") => "bool",
        (
            TargetLang::Dart,
            "INT" | "INTEGER" | "SMALLINT" | "BIGINT" | "INT2" | "INT4" | "INT8" | "INT64"
            | "UINT64" | "SERIAL" | "BIGSERIAL",
        ) => "int",
        (
            TargetLang::Dart,
            "FLOAT" | "FLOAT4" | "FLOAT8" | "DOUBLE" | "REAL" | "NUMERIC" | "DECIMAL",
        ) => "double",
        (TargetLang::Dart, "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME" | "DATE" | "TIME") => {
            "DateTime"
        },
        (TargetLang::Dart, "JSON" | "JSONB") => "Map<String, Object?>",
        (TargetLang::Dart, "FILE") => "File",
        (TargetLang::Dart, "BYTES" | "BYTEA" | "BLOB" | "BINARY") => "String",
        (TargetLang::Dart, "EMBEDDING") => "List<double>",
        (TargetLang::Dart, _) => "String",

        (TargetLang::Rust, "BOOLEAN" | "BOOL") => "bool",
        (TargetLang::Rust, "SMALLINT" | "INT2") => "i16",
        (TargetLang::Rust, "INT" | "INTEGER" | "INT4" | "SERIAL") => "i32",
        (TargetLang::Rust, "BIGINT" | "INT8" | "INT64" | "BIGSERIAL") => "i64",
        (TargetLang::Rust, "FLOAT" | "FLOAT4" | "REAL") => "f32",
        (TargetLang::Rust, "DOUBLE" | "FLOAT8") => "f64",
        (TargetLang::Rust, "BYTES" | "BYTEA" | "BLOB" | "BINARY") => "Vec<u8>",
        (TargetLang::Rust, "JSON" | "JSONB") => "serde_json::Value",
        (TargetLang::Rust, "FILE") => "File",
        (TargetLang::Rust, _) => "String",

        (TargetLang::Python, "BOOLEAN" | "BOOL") => "bool",
        (
            TargetLang::Python,
            "SMALLINT" | "INT2" | "INT" | "INTEGER" | "INT4" | "SERIAL" | "BIGINT" | "INT8"
            | "INT64" | "UINT64" | "BIGSERIAL",
        ) => "int",
        (TargetLang::Python, "FLOAT" | "FLOAT4" | "FLOAT8" | "DOUBLE" | "REAL") => "float",
        // Kept lossless as text, matching TypeScript and Rust.
        (TargetLang::Python, "DECIMAL" | "NUMERIC") => "str",
        (TargetLang::Python, "BYTES" | "BYTEA" | "BLOB" | "BINARY") => "bytes",
        (TargetLang::Python, "JSON" | "JSONB") => "Any",
        // Qualified through the `dt` alias so the generated module's `date`
        // and `time` column factories cannot shadow the stdlib types.
        (TargetLang::Python, "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME") => "dt.datetime",
        (TargetLang::Python, "DATE") => "dt.date",
        (TargetLang::Python, "TIME") => "dt.time",
        (TargetLang::Python, "EMBEDDING") => "List[float]",
        (TargetLang::Python, "FILE") => "FileRef",
        (TargetLang::Python, _) => "str",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_timestamps_match_drizzle_date_mode() {
        assert_eq!(builtin("TIMESTAMP", TargetLang::TypeScript), "Date");
        assert_eq!(builtin("TIMESTAMPTZ", TargetLang::TypeScript), "Date");
        assert_eq!(builtin("DATETIME", TargetLang::TypeScript), "Date");
        assert_eq!(builtin("TEXT", TargetLang::TypeScript), "string");
        assert_eq!(builtin("FILE", TargetLang::TypeScript), "File");
    }

    #[test]
    fn python_builtins_match_the_runtime_codecs() {
        for (sql, expected) in [
            ("BOOLEAN", "bool"),
            ("SMALLINT", "int"),
            ("INT", "int"),
            ("BIGINT", "int"),
            ("DOUBLE", "float"),
            ("REAL", "float"),
            // Lossless as text, the same choice TypeScript and Rust make.
            ("DECIMAL", "str"),
            ("NUMERIC", "str"),
            ("TEXT", "str"),
            ("UUID", "str"),
            ("BYTES", "bytes"),
            ("JSON", "Any"),
            ("JSONB", "Any"),
            ("FILE", "FileRef"),
            ("EMBEDDING(384)", "List[float]"),
            // Qualified so the date/time column factories cannot shadow them.
            ("TIMESTAMP", "dt.datetime"),
            ("TIMESTAMPTZ", "dt.datetime"),
            ("DATE", "dt.date"),
            ("TIME", "dt.time"),
        ] {
            assert_eq!(builtin(sql, TargetLang::Python), expected, "{sql}");
        }
    }
}
