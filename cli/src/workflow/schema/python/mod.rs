//! Python bindings generated from a [`ContractSnapshot`].
//!
//! Tables are declared in `schema.sql`; this emits the row dataclasses, typed
//! columns, and `KTable` declarations that call into the hand-written runtime
//! in `kalamdb.schema`. Nothing here emits DDL or validates table options:
//! those belong to `schema.sql` and the server.

mod rows;
mod tables;

use std::collections::BTreeSet;

use kalamdb_sql::contracts::{ContractField, ContractSnapshot, ContractTypeKind};
use serde::Serialize;

use self::{
    rows::{assign_unique_field_names, escape_python_string, PyColumn, SDK_ALIAS},
    tables::{table_bindings, ColumnsClassContext, TableSpecContext},
};
use crate::{
    error::Result,
    workflow::{
        project::templates::render_schema_gen_file,
        schema::{
            naming::{contract_hash_line, screaming_snake_case, AssignedNames},
            output::{
                canonical_row_type_id, line_docs, row_type_ident, type_comment, write_text,
                SchemaEmitInput,
            },
            procedures::ProcedureCatalog,
            types::{render_field_type, TargetLang},
        },
    },
};

const PYTHON_TEMPLATE: &str = "generated/kalam.py";

/// Runtime names the generated module may import. Kept explicit so the import
/// list is exactly what the file uses, which keeps linters quiet.
const RUNTIME_SYMBOLS: &[&str] = &[
    "Column",
    "FileRef",
    "KTable",
    "Table",
    "as_bool",
    "as_bool_or_none",
    "as_bytes",
    "as_bytes_or_none",
    "as_date",
    "as_date_or_none",
    "as_datetime",
    "as_datetime_or_none",
    "as_enum",
    "as_enum_or_none",
    "as_file_ref",
    "as_file_ref_or_none",
    "as_float",
    "as_float_list",
    "as_float_or_none",
    "as_int",
    "as_int_or_none",
    "as_json",
    "as_list",
    "as_list_or_none",
    "as_str",
    "as_str_or_none",
    "as_struct",
    "as_struct_or_none",
    "as_time",
    "as_time_or_none",
    "bigint",
    "boolean",
    "bytes_",
    "date",
    "double",
    "embedding",
    "encode_date",
    "encode_datetime",
    "encode_enum",
    "encode_file",
    "encode_list",
    "encode_row",
    "encode_time",
    "file",
    "integer",
    "json",
    "smallint",
    "text",
    "time",
    "timestamp",
    "uuid",
];

#[derive(Serialize)]
struct PythonContext {
    contract_hash_line: String,
    typing_imports: String,
    needs_datetime: bool,
    needs_enum: bool,
    needs_dataclass: bool,
    sdk_imports: Vec<String>,
    /// Columns classes call the runtime through this alias, so a generated
    /// attribute can never shadow a factory.
    sdk_alias: &'static str,
    needs_sdk_alias: bool,
    enums: Vec<EnumContext>,
    dataclasses: Vec<DataclassContext>,
    aliases: Vec<AliasContext>,
    columns_classes: Vec<ColumnsClassContext>,
    tables: Vec<TableSpecContext>,
    has_tables: bool,
    exports: Vec<String>,
}

#[derive(Serialize)]
struct EnumContext {
    doc: String,
    ident: String,
    body: String,
}

#[derive(Serialize)]
struct DataclassContext {
    doc: String,
    ident: String,
    fields: String,
    decode: String,
    encode: String,
}

#[derive(Serialize)]
struct AliasContext {
    doc: String,
    ident: String,
    target: String,
}

pub fn write_python_schema(input: &SchemaEmitInput<'_>) -> Result<()> {
    write_text(
        input.output_path,
        &render_python_source(input.snapshot, input.hash, input.names, input.procedures)?,
    )?;
    ensure_package_marker(input)
}

/// Make the output directory an importable package.
///
/// Unlike the other targets, Python resolves imports by package: without an
/// `__init__.py`, `generated/kalam.py` imports at runtime as a namespace package
/// but type checkers cannot map the path to a module name. An existing file is
/// never touched, and nothing is written when the module sits at the project
/// root.
fn ensure_package_marker(input: &SchemaEmitInput<'_>) -> Result<()> {
    let Some(parent) = input.output_path.parent() else {
        return Ok(());
    };
    if parent == input.project_root || parent.as_os_str().is_empty() {
        return Ok(());
    }
    let marker = parent.join("__init__.py");
    if marker.exists() {
        return Ok(());
    }
    write_text(&marker, "\"\"\"Generated KalamDB bindings.\"\"\"\n")
}

/// Pure variant for tests, mirroring `generate_dart_source` and
/// `generate_rust_source`.
pub fn generate_python_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
) -> Result<String> {
    let procedures = ProcedureCatalog::from_snapshot(snapshot, names);
    render_python_source(snapshot, hash, names, &procedures)
}

fn render_python_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> Result<String> {
    render_schema_gen_file(
        "python",
        PYTHON_TEMPLATE,
        &python_context(snapshot, hash, names, procedures),
    )
}

fn python_context(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> PythonContext {
    let mut enums = Vec::new();
    let mut dataclasses = Vec::new();
    let mut aliases = Vec::new();
    // Annotations drive the typing imports; code drives the runtime imports.
    // They stay separate so a `dt.date` annotation cannot be mistaken for the
    // `date` column factory.
    let mut emitted: Vec<String> = Vec::new();
    let mut code: Vec<String> = Vec::new();

    for (id, ty) in &snapshot.types {
        match &ty.kind {
            ContractTypeKind::Enum { labels } => {
                let mut body = String::new();
                let mut used = BTreeSet::new();
                for label in labels {
                    let base = screaming_snake_case(label);
                    // A trailing underscore would make `_2FA` into `_2FA_`, and
                    // Enum reserves `_name_` for itself. A numeric suffix keeps
                    // the member importable.
                    let mut member = base.clone();
                    let mut index = 2u32;
                    while !used.insert(member.clone()) {
                        member = format!("{base}_{index}");
                        index += 1;
                    }
                    body.push_str(&format!(
                        "    {member} = \"{}\"\n",
                        escape_python_string(label)
                    ));
                }
                enums.push(EnumContext {
                    doc: docstring(ty.comment.as_deref()),
                    ident: names.type_ident(id).to_string(),
                    body: body.trim_end().to_string(),
                });
            },
            ContractTypeKind::Composite { fields } => {
                let fields: Vec<ContractField> =
                    fields.iter().filter(|field| !field.dropped).cloned().collect();
                dataclasses.push(dataclass_context(
                    names.type_ident(id),
                    &fields,
                    names,
                    snapshot,
                    ty.comment.as_deref(),
                    &mut emitted,
                    &mut code,
                ));
            },
            ContractTypeKind::Domain { base_type, .. } => {
                let target = render_field_type(base_type, names, TargetLang::Python);
                emitted.push(target.clone());
                aliases.push(AliasContext {
                    doc: line_docs(ty.comment.as_deref(), "# "),
                    ident: names.type_ident(id).to_string(),
                    target,
                });
            },
            ContractTypeKind::ImplicitTableRow { .. }
            | ContractTypeKind::RowAlias { .. }
            | ContractTypeKind::TopicPayload { .. } => {},
        }
    }

    for table in snapshot.tables.values() {
        let canonical_id = canonical_row_type_id(table).to_string();
        let fields: Vec<ContractField> =
            table.fields.iter().filter(|field| !field.dropped).cloned().collect();
        dataclasses.push(dataclass_context(
            names.type_ident(&canonical_id),
            &fields,
            names,
            snapshot,
            type_comment(snapshot, &canonical_id)
                .or_else(|| type_comment(snapshot, table.row_type_id.as_str())),
            &mut emitted,
            &mut code,
        ));
        if let Some(alias) = &table.row_alias_id {
            let alias_ident = names.type_ident(alias.as_str());
            let row_ident = names.type_ident(table.row_type_id.as_str());
            if alias_ident != row_ident {
                aliases.push(AliasContext {
                    doc: String::new(),
                    ident: row_ident.to_string(),
                    target: alias_ident.to_string(),
                });
            }
        }
    }

    // Topic payloads union the row types that publish to the topic.
    for (id, ty) in &snapshot.types {
        if let ContractTypeKind::TopicPayload { sources, .. } = &ty.kind {
            let variants: Vec<String> = sources
                .iter()
                .map(|table_id| row_type_ident(snapshot, names, table_id).to_string())
                .collect();
            let target = match variants.len() {
                0 => "Any".to_string(),
                1 => variants[0].clone(),
                _ => format!("Union[{}]", variants.join(", ")),
            };
            emitted.push(target.clone());
            aliases.push(AliasContext {
                doc: line_docs(ty.comment.as_deref(), "# "),
                ident: names.type_ident(id).to_string(),
                target,
            });
        }
    }

    // Procedure request and result types only; the call client is not generated
    // yet, matching the Rust target.
    for spec in procedures.procedures() {
        dataclasses.push(dataclass_context(
            &spec.rust_input_ident(),
            spec.parameters(),
            names,
            snapshot,
            spec.comment(),
            &mut emitted,
            &mut code,
        ));
        // A void routine gets no result alias: `X = None` is a value, not a
        // usable type, so exporting one would mislead callers.
        if let Some(ret) = spec.return_type() {
            let target = render_field_type(ret, names, TargetLang::Python);
            emitted.push(target.clone());
            aliases.push(AliasContext {
                doc: String::new(),
                ident: spec.rust_output_ident(),
                target,
            });
        }
    }

    let bindings = table_bindings(snapshot, names);
    code.extend(bindings.exprs.iter().cloned());
    // `Table` annotates the registry whether or not any table exists; `KTable`
    // is only called when one does.
    code.push("Table".to_string());
    if !bindings.tables.is_empty() {
        code.push("KTable".to_string());
    }
    // FileRef only ever appears as an annotation, so carry it over by hand.
    if emitted.iter().any(|text| text.contains("FileRef")) {
        code.push("FileRef".to_string());
    }

    let mut exports: BTreeSet<String> = BTreeSet::new();
    exports.extend(enums.iter().map(|item| item.ident.clone()));
    exports.extend(dataclasses.iter().map(|item| item.ident.clone()));
    exports.extend(aliases.iter().map(|item| item.ident.clone()));
    exports.extend(bindings.columns_classes.iter().map(|item| item.ident.clone()));
    exports.extend(bindings.tables.iter().map(|item| item.var_name.clone()));
    exports.insert("KALAM_TABLES".to_string());

    let has_tables = !bindings.tables.is_empty();
    let needs_dataclass = !dataclasses.is_empty();

    PythonContext {
        contract_hash_line: contract_hash_line(hash),
        typing_imports: typing_imports(&emitted, needs_dataclass, has_tables),
        needs_datetime: emitted.iter().any(|text| text.contains("dt.")),
        needs_enum: !enums.is_empty(),
        needs_dataclass,
        sdk_imports: sdk_imports(&code),
        sdk_alias: SDK_ALIAS,
        needs_sdk_alias: !bindings.columns_classes.is_empty(),
        enums,
        dataclasses,
        aliases,
        columns_classes: bindings.columns_classes,
        tables: bindings.tables,
        has_tables,
        exports: exports.into_iter().collect(),
    }
}

fn dataclass_context(
    ident: &str,
    fields: &[ContractField],
    names: &AssignedNames,
    snapshot: &ContractSnapshot,
    comment: Option<&str>,
    emitted: &mut Vec<String>,
    code: &mut Vec<String>,
) -> DataclassContext {
    let mut columns: Vec<PyColumn> =
        fields.iter().map(|field| PyColumn::from_field(field, names, snapshot)).collect();
    assign_unique_field_names(&mut columns);

    let mut annotations = String::new();
    let mut decode = String::new();
    let mut encode = String::new();
    for column in &columns {
        annotations.push_str(&format!("    {}: {}\n", column.field_name, column.py_type));
        decode.push_str(&format!("            {}={},\n", column.field_name, column.decode()));
        encode.push_str(&format!(
            "            \"{}\": {},\n",
            escape_python_string(&column.sql_name),
            column.encode()
        ));
        emitted.push(column.py_type.clone());
        code.push(column.decode());
        code.push(column.encode());
    }

    if columns.is_empty() {
        // A row type with no readable fields still has to be a valid class.
        annotations.push_str("    pass\n");
    }

    DataclassContext {
        doc: docstring(comment),
        ident: ident.to_string(),
        fields: annotations.trim_end().to_string(),
        decode: decode.trim_end().to_string(),
        encode: encode.trim_end().to_string(),
    }
}

/// A class-level docstring, or nothing when the type carries no comment.
fn docstring(comment: Option<&str>) -> String {
    let text = line_docs(comment, "");
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.lines().count() == 1 {
        format!("    \"\"\"{}\"\"\"\n", docstring_text(trimmed))
    } else {
        let body: Vec<String> =
            trimmed.lines().map(|line| format!("    {}", docstring_text(line))).collect();
        format!("    \"\"\"\n{}\n    \"\"\"\n", body.join("\n"))
    }
}

/// Comment text that is safe between triple quotes. Backslashes are doubled
/// first: a comment ending in one would otherwise escape the closing delimiter
/// and the module would not compile. Quotes become apostrophes so the delimiter
/// itself can never appear.
fn docstring_text(line: &str) -> String {
    line.replace('\\', "\\\\").replace('"', "'")
}

fn typing_imports(emitted: &[String], needs_dataclass: bool, has_tables: bool) -> String {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    let joined = emitted.join(" ");
    if joined.contains("Optional[") {
        names.insert("Optional");
    }
    if joined.contains("List[") {
        names.insert("List");
    }
    if joined.contains("Union[") {
        names.insert("Union");
    }
    // `KALAM_TABLES: Dict[str, Table[Any]]` is emitted even when a schema
    // declares no tables, so its types are always imported.
    let _ = has_tables;
    names.insert("Any");
    names.insert("Dict");
    if needs_dataclass {
        names.insert("Mapping");
    }
    if names.is_empty() {
        return String::new();
    }
    format!("from typing import {}", names.into_iter().collect::<Vec<_>>().join(", "))
}

/// Runtime symbols the file references, as whole identifiers.
fn sdk_imports(emitted: &[String]) -> Vec<String> {
    let mut tokens: BTreeSet<String> = BTreeSet::new();
    for text in emitted {
        let mut current = String::new();
        for ch in text.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                current.push(ch);
            } else {
                if !current.is_empty() {
                    tokens.insert(std::mem::take(&mut current));
                }
            }
        }
        if !current.is_empty() {
            tokens.insert(current);
        }
    }
    RUNTIME_SYMBOLS
        .iter()
        .filter(|symbol| tokens.contains(**symbol))
        .map(|symbol| (*symbol).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use kalamdb_sql::compile_contract_sql;

    use super::*;
    use crate::workflow::schema::{
        naming::{assign_names, NamingOptions},
        procedures::ProcedureCatalog,
    };

    const GOLDEN_SQL: &str = r#"
CREATE SCHEMA chat;
CREATE TYPE chat.status AS ENUM ('active', 'blocked');
CREATE TYPE chat.address AS (city TEXT, country TEXT);
CREATE USER TABLE chat.profiles (
  id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
  email TEXT NOT NULL,
  status chat.status NOT NULL,
  address chat.address,
  tags TEXT[],
  avatar FILE,
  score DOUBLE,
  seen_on DATE,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
);
CREATE SHARED TABLE chat.rooms (id BIGINT PRIMARY KEY, name TEXT NOT NULL);
CREATE STREAM TABLE chat.events (id BIGINT PRIMARY KEY, payload JSON);
CREATE TABLE chat.plain (id BIGINT PRIMARY KEY);
"#;

    fn generate(sql: &str) -> String {
        generate_with(sql, false)
    }

    fn generate_with(sql: &str, unqualified_names: bool) -> String {
        let snapshot = compile_contract_sql(sql, "public").unwrap();
        let hash = kalamdb_sql::canonical_contract_hash(&snapshot);
        let names = assign_names(&snapshot, NamingOptions { unqualified_names }).unwrap();
        generate_python_source(&snapshot, &hash, &names).unwrap()
    }

    #[test]
    fn generates_dataclasses_and_table_declarations_from_sql() {
        let source = generate(GOLDEN_SQL);
        assert!(source.starts_with("# Generated by kalam schema gen. Do not edit."));
        assert!(source.contains("# contract_hash: "));
        assert!(source.contains("from __future__ import annotations"));
        assert!(source.contains("from kalamdb.schema import ("));
        assert!(source.contains("@dataclass(frozen=True)\nclass ChatProfiles:"));
        assert!(source.contains("    id: int\n"));
        assert!(source.contains("    email: str\n"));
        assert!(source.contains("    created_at: dt.datetime\n"));
        assert!(source.contains("    def from_row(cls, row: Mapping[str, Any]) -> ChatProfiles:"));
        assert!(source.contains("            id=as_int(row.get(\"id\")),\n"));
        assert!(source.contains("    def to_row(self) -> Dict[str, Any]:"));
        assert!(source.contains("row=ChatProfiles,"));
        assert!(!source.to_lowercase().contains("placeholder"));
    }

    #[test]
    fn no_ddl_or_table_options_are_emitted() {
        // Comments are skipped: the header explains that options live in
        // schema.sql, so it names them on purpose.
        let source = generate(GOLDEN_SQL)
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .to_uppercase();
        for forbidden in [
            "CREATE TABLE",
            "TTL_SECONDS",
            "STORAGE_ID",
            "FLUSH_POLICY",
            "COMPRESSION",
            "EVICTION_STRATEGY",
            "MAX_STREAM_SIZE_BYTES",
            "USE_USER_STORAGE",
            "DELETED_RETENTION_HOURS",
        ] {
            assert!(!source.contains(forbidden), "generated python mentions {forbidden}");
        }
    }

    #[test]
    fn table_kind_selects_the_ktable_factory() {
        let source = generate(GOLDEN_SQL);
        assert!(source.contains("KTable.user(\n    \"chat.profiles\","));
        assert!(source.contains("KTable.shared(\n    \"chat.rooms\","));
        assert!(source.contains("KTable.stream(\n    \"chat.events\","));
        // A table declared without a kind gets the plain factory.
        assert!(source.contains("KTable.table(\n    \"chat.plain\","));
    }

    #[test]
    fn enums_emit_str_enum_with_wire_values() {
        let source = generate(GOLDEN_SQL);
        assert!(source.contains("class ChatStatus(str, Enum):"));
        assert!(source.contains("    ACTIVE = \"active\""));
        assert!(source.contains("    BLOCKED = \"blocked\""));
        assert!(source.contains("status=as_enum(row.get(\"status\"), ChatStatus)"));
        assert!(source.contains("\"status\": encode_enum(self.status)"));
    }

    #[test]
    fn nullable_array_file_and_struct_columns_decode() {
        let source = generate(GOLDEN_SQL);
        assert!(source.contains("    tags: Optional[List[str]]\n"));
        assert!(source.contains("tags=as_list_or_none(row.get(\"tags\"), as_str)"));
        assert!(source.contains("    avatar: Optional[FileRef]\n"));
        assert!(source.contains("avatar=as_file_ref_or_none(row.get(\"avatar\"))"));
        assert!(source.contains("\"avatar\": encode_file(self.avatar)"));
        assert!(source.contains("    address: Optional[ChatAddress]\n"));
        assert!(source.contains("address=as_struct_or_none(row.get(\"address\"), ChatAddress)"));
        assert!(source.contains("\"address\": encode_row(self.address)"));
    }

    #[test]
    fn temporal_columns_use_the_datetime_alias() {
        let source = generate(GOLDEN_SQL);
        assert!(source.contains("import datetime as dt"));
        assert!(source.contains("    seen_on: Optional[dt.date]\n"));
        assert!(source.contains("seen_on=as_date_or_none(row.get(\"seen_on\"))"));
        assert!(source.contains("\"created_at\": encode_datetime(self.created_at)"));
        // The `date`/`time` column factories must not shadow the stdlib types.
        assert!(!source.contains("from datetime import"));
    }

    #[test]
    fn string_literals_escape_the_double_quote_delimiter() {
        let source = generate("CREATE TYPE mood AS ENUM ('say \"hi\"');");
        assert!(source.contains(r#"SAY_HI = "say \"hi\"""#), "{source}");
    }

    #[test]
    fn colliding_enum_members_keep_a_digit_suffix() {
        // `_2FA_` would be a sunder name and Enum rejects it at import.
        let source = generate("CREATE TYPE mood AS ENUM ('a b', 'a_b');");
        assert!(source.contains("    A_B = \"a b\""), "{source}");
        assert!(source.contains("    A_B_2 = \"a_b\""), "{source}");
        assert!(!source.contains("A_B_ ="), "{source}");
    }

    #[test]
    fn colliding_field_names_stay_distinct_and_keep_their_sql_keys() {
        let source = generate(
            "CREATE TABLE t (id BIGINT PRIMARY KEY, user_id TEXT, \"user__id\" TEXT);",
        );
        assert!(source.contains("    user_id: Optional[str]"), "{source}");
        assert!(source.contains("    user_id2: Optional[str]"), "{source}");
        assert!(source.contains("user_id2=as_str_or_none(row.get(\"user__id\"))"), "{source}");
        assert!(source.contains("\"user__id\": self.user_id2"), "{source}");
    }

    #[test]
    fn a_column_never_takes_a_generated_method_name() {
        let source =
            generate("CREATE TABLE t (id BIGINT PRIMARY KEY, \"to_row\" TEXT);");
        assert!(source.contains("    to_row2: Optional[str]"), "{source}");
        assert!(source.contains("\"to_row\": self.to_row2"), "{source}");
    }

    #[test]
    fn a_table_never_replaces_a_runtime_import() {
        let source = generate("CREATE TABLE \"as_int\" (id BIGINT PRIMARY KEY);");
        assert!(source.contains("as_int2: Table[AsInt] = KTable."), "{source}");
        assert!(source.contains("as_int(row.get(\"id\"))"), "{source}");
    }

    #[test]
    fn columns_classes_call_factories_through_the_alias() {
        // Evaluated in a class body, so an unqualified `text(...)` would hit
        // the `text` column declared on the line above it.
        let source = generate("CREATE TABLE t (\"text\" TEXT PRIMARY KEY, title TEXT);");
        assert!(source.contains("from kalamdb import schema as _kalam"), "{source}");
        assert!(source.contains("    text: Column[str] = _kalam.text(\"text\")"), "{source}");
        assert!(source.contains("    title: Column[str] = _kalam.text(\"title\")"), "{source}");
    }

    #[test]
    fn an_empty_schema_still_imports_the_registry_types() {
        let source = generate("CREATE SCHEMA empty_app;");
        assert!(source.contains("KALAM_TABLES: Dict[str, Table[Any]] = {}"), "{source}");
        assert!(source.contains("from typing import Any, Dict"), "{source}");
        assert!(source.contains("    Table,"), "{source}");
    }

    #[test]
    fn docstrings_escape_a_trailing_backslash() {
        // Left alone, the backslash would escape the first quote of the closing
        // delimiter and the module would not compile.
        let source = generate(
            "CREATE TYPE postal AS (city TEXT);\nCOMMENT ON TYPE postal IS 'ends with \\';",
        );
        assert!(source.contains(r#"    """ends with \\""""#), "{source}");
    }

    #[test]
    fn columns_classes_declare_keys_and_server_defaults() {
        let source = generate(GOLDEN_SQL);
        assert!(source.contains("class ChatProfilesColumns:"));
        assert!(source.contains("\"\"\"Typed columns for chat.profiles.\"\"\""));
        assert!(source
            .contains("    id: Column[int] = _kalam.bigint(\"id\").primary_key().not_null().server_default()"));
        assert!(source.contains("    email: Column[str] = _kalam.text(\"email\").not_null()"));
        assert!(source.contains("    avatar: Column[FileRef] = _kalam.file(\"avatar\")"));
        assert!(source.contains("    tags: Column[List[str]] = _kalam.Column(\"tags\", \"TEXT[]\")"));
        assert!(source
            .contains("    status: Column[ChatStatus] = _kalam.Column(\"status\", \"chat.status\").not_null()"));
    }

    #[test]
    fn single_primary_keys_omit_the_ordinal() {
        // The contract compiler rejects composite PRIMARY KEYs ("Composite
        // PRIMARY KEYs are not supported yet"), so generated columns never
        // need an ordinal. `Column.primary_key(ordinal)` in the runtime is
        // ready for when they land.
        let source = generate("CREATE STREAM TABLE log (seq BIGINT PRIMARY KEY);");
        assert!(source.contains("    seq: Column[int] = _kalam.bigint(\"seq\").primary_key().not_null()"));
        assert!(!source.contains(".primary_key(1)"));
    }

    #[test]
    fn keyless_tables_still_get_a_declaration() {
        // Dart omits keyless tables from KalamTables; Python declares every
        // table and refuses only keyed operations at runtime.
        let source = generate("CREATE STREAM TABLE log (line TEXT NOT NULL);");
        assert!(source.contains("KTable.stream("), "{source}");
        assert!(!source.contains(".primary_key()"));
    }

    #[test]
    fn row_alias_shares_one_dataclass() {
        let source = generate(
            "CREATE SCHEMA chat;\nCREATE USER TABLE chat.users (id TEXT PRIMARY KEY) \
             ROW TYPE chat.user;",
        );
        assert!(source.contains("class ChatUser:"));
        assert!(!source.contains("class ChatUsers:"));
        assert!(source.contains("ChatUsers = ChatUser"));
    }

    #[test]
    fn keyword_columns_get_a_trailing_underscore() {
        let source =
            generate("CREATE TABLE t (id TEXT PRIMARY KEY, \"class\" TEXT, \"from\" TEXT);");
        assert!(source.contains("    class_: Optional[str]\n"), "{source}");
        assert!(source.contains("    from_: Optional[str]\n"), "{source}");
        // The SQL name is carried separately, so the column keeps its real name.
        assert!(source.contains("class_=as_str_or_none(row.get(\"class\"))"));
        assert!(source.contains("\"class\": self.class_"));
        assert!(source.contains("    class_: Column[str] = _kalam.text(\"class\")"));
    }

    #[test]
    fn imports_list_only_what_the_file_uses() {
        let source = generate("CREATE TABLE t (id TEXT PRIMARY KEY, label TEXT NOT NULL);");
        assert!(source.contains("    as_str,\n"));
        // No enums, composites, files or temporals in this schema.
        assert!(!source.contains("as_enum"));
        assert!(!source.contains("as_struct"));
        assert!(!source.contains("as_file_ref"));
        assert!(!source.contains("import datetime as dt"));
        assert!(!source.contains("from enum import Enum"));
        // The import block stays sorted so --check stays byte stable.
        let block = source.split("from kalamdb.schema import (").nth(1).unwrap();
        let names: Vec<&str> = block
            .split(")\n")
            .next()
            .unwrap()
            .lines()
            .map(|line| line.trim().trim_end_matches(','))
            .filter(|line| !line.is_empty())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "runtime imports are not sorted");
    }

    #[test]
    fn empty_schema_emits_an_importable_module() {
        let source = generate("CREATE SCHEMA empty_app;");
        assert!(source.contains("KALAM_TABLES: Dict[str, Table[Any]] = {}"));
        assert!(!source.contains("from kalamdb.schema import ()"));
        assert!(source.contains("__all__ = ["));
    }

    #[test]
    fn comments_become_docstrings() {
        let source = generate(
            "CREATE TYPE postal AS (city TEXT);\nCOMMENT ON TYPE postal IS 'Postal address';",
        );
        assert!(source.contains("    \"\"\"Postal address\"\"\""), "{source}");
    }

    #[test]
    fn unqualified_names_emit_short_idents() {
        let source = generate_with(
            "CREATE SCHEMA chat;\nCREATE USER TABLE chat.profiles (id TEXT PRIMARY KEY);",
            true,
        );
        assert!(source.contains("class Profiles:"), "{source}");
        assert!(source.contains("profiles: Table[Profiles] = KTable.user("));
    }

    #[test]
    fn output_is_byte_stable_across_runs() {
        assert_eq!(generate(GOLDEN_SQL), generate(GOLDEN_SQL));
    }

    #[test]
    fn write_python_schema_creates_parent_directories() {
        let temp = tempfile::TempDir::new().unwrap();
        let output = temp.path().join("generated/kalam.py");
        let snapshot = compile_contract_sql(
            "CREATE TABLE todos (id TEXT PRIMARY KEY, title TEXT NOT NULL);",
            "public",
        )
        .unwrap();
        let hash = kalamdb_sql::canonical_contract_hash(&snapshot);
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let procedures = ProcedureCatalog::from_snapshot(&snapshot, &names);
        write_python_schema(&SchemaEmitInput {
            project_root: temp.path(),
            output_path: &output,
            snapshot: &snapshot,
            hash: &hash,
            names: &names,
            procedures: &procedures,
        })
        .unwrap();
        let written = std::fs::read_to_string(&output).unwrap();
        assert!(written.contains("KTable.table("));
        assert!(written.contains("row=Todos,"));
    }

    #[test]
    fn output_directory_becomes_an_importable_package() {
        let temp = tempfile::TempDir::new().unwrap();
        let output = temp.path().join("generated/kalam.py");
        let snapshot =
            compile_contract_sql("CREATE TABLE todos (id TEXT PRIMARY KEY);", "public").unwrap();
        let hash = kalamdb_sql::canonical_contract_hash(&snapshot);
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let procedures = ProcedureCatalog::from_snapshot(&snapshot, &names);
        let input = SchemaEmitInput {
            project_root: temp.path(),
            output_path: &output,
            snapshot: &snapshot,
            hash: &hash,
            names: &names,
            procedures: &procedures,
        };
        write_python_schema(&input).unwrap();

        // Without this, type checkers cannot map generated/kalam.py to a module.
        let marker = temp.path().join("generated/__init__.py");
        assert!(marker.is_file(), "missing package marker");

        // An existing marker is never overwritten.
        std::fs::write(&marker, "# hand written\n").unwrap();
        write_python_schema(&input).unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "# hand written\n");
    }

    #[test]
    fn a_root_level_module_needs_no_package_marker() {
        let temp = tempfile::TempDir::new().unwrap();
        let output = temp.path().join("kalam_schema.py");
        let snapshot =
            compile_contract_sql("CREATE TABLE todos (id TEXT PRIMARY KEY);", "public").unwrap();
        let hash = kalamdb_sql::canonical_contract_hash(&snapshot);
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let procedures = ProcedureCatalog::from_snapshot(&snapshot, &names);
        write_python_schema(&SchemaEmitInput {
            project_root: temp.path(),
            output_path: &output,
            snapshot: &snapshot,
            hash: &hash,
            names: &names,
            procedures: &procedures,
        })
        .unwrap();
        assert!(!temp.path().join("__init__.py").exists());
    }
}
