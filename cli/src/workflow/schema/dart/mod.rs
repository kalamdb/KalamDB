//! Dart/Flutter schema generation from a contract snapshot.

use kalamdb_sql::contracts::{ContractSnapshot, ContractTypeKind};
use serde::Serialize;

use crate::{
    error::Result,
    workflow::{
        project::templates::{escape_js_single_quoted_string, render_schema_gen_file},
        schema::{
            naming::{camel_case, contract_hash_line, AssignedNames},
            output::{canonical_row_type_id, line_docs, type_comment, write_text, SchemaEmitInput},
            procedures::ProcedureCatalog,
        },
    },
};

mod functions;
mod rows;
mod tables;

use functions::{functions_context, procedure_types, ResultAlias};
use rows::{row_class_context, RowClassContext};
use tables::{table_specs, TableSpecContext};

const DART_TEMPLATE: &str = "lib/generated/kalam.dart";

#[derive(Serialize)]
struct DartContext {
    contract_hash_line: String,
    enums:              Vec<EnumContext>,
    classes:            Vec<RowClassContext>,
    aliases:            Vec<AliasContext>,
    result_aliases:     Vec<ResultAlias>,
    tables:             Vec<TableSpecContext>,
    has_functions:      bool,
    namespaces:         Vec<functions::NamespaceContext>,
    needs_helpers:      bool,
}

#[derive(Serialize)]
struct EnumContext {
    doc:    String,
    ident:  String,
    labels: Vec<EnumLabel>,
}

#[derive(Serialize)]
struct EnumLabel {
    ident:      String,
    label:      String,
    terminator: &'static str,
}

#[derive(Serialize)]
struct AliasContext {
    ident:    String,
    alias_of: String,
}

pub fn write_dart_schema(input: &SchemaEmitInput<'_>) -> Result<()> {
    write_text(
        input.output_path,
        &render_dart_source(input.snapshot, input.hash, input.names, input.procedures)?,
    )
}

pub fn generate_dart_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
) -> Result<String> {
    let procedures = ProcedureCatalog::from_snapshot(snapshot, names);
    render_dart_source(snapshot, hash, names, &procedures)
}

fn render_dart_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> Result<String> {
    render_schema_gen_file("dart", DART_TEMPLATE, &dart_context(snapshot, hash, names, procedures))
}

fn dart_context(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> DartContext {
    let functions = functions_context(snapshot, names, procedures);
    let (mut classes, result_aliases) = procedure_types(snapshot, names, procedures);
    let mut named_classes = Vec::new();
    let mut enums = Vec::new();
    for (id, ty) in &snapshot.types {
        match &ty.kind {
            ContractTypeKind::Enum { labels } => {
                let last = labels.len().saturating_sub(1);
                enums.push(EnumContext {
                    doc:    line_docs(ty.comment.as_deref(), "/// "),
                    ident:  names.type_ident(id).to_string(),
                    labels: labels
                        .iter()
                        .enumerate()
                        .map(|(index, label)| EnumLabel {
                            ident:      camel_case(label),
                            label:      escape_js_single_quoted_string(label),
                            terminator: if index == last { ";" } else { "," },
                        })
                        .collect(),
                });
            },
            ContractTypeKind::Composite { fields } => {
                named_classes.push(row_class_context(
                    names.type_ident(id),
                    fields,
                    names,
                    snapshot,
                    ty.comment.as_deref(),
                    true,
                ));
            },
            ContractTypeKind::ImplicitTableRow { .. }
            | ContractTypeKind::RowAlias { .. }
            | ContractTypeKind::TopicPayload { .. } => {},
        }
    }

    let mut aliases = Vec::new();
    for table in snapshot.tables.values() {
        let canonical_id = canonical_row_type_id(table).to_string();
        named_classes.push(row_class_context(
            names.type_ident(&canonical_id),
            &table.fields,
            names,
            snapshot,
            type_comment(snapshot, &canonical_id)
                .or_else(|| type_comment(snapshot, table.row_type_id.as_str())),
            true,
        ));
        if let Some(alias) = &table.row_alias_id {
            let alias_ident = names.type_ident(alias.as_str());
            let row_ident = names.type_ident(table.row_type_id.as_str());
            if alias_ident != row_ident {
                aliases.push(AliasContext {
                    ident:    row_ident.to_string(),
                    alias_of: alias_ident.to_string(),
                });
            }
        }
    }
    named_classes.append(&mut classes);

    DartContext {
        contract_hash_line: contract_hash_line(hash),
        enums,
        classes: named_classes,
        aliases,
        result_aliases,
        tables: table_specs(snapshot, names),
        has_functions: !procedures.is_empty(),
        namespaces: functions.namespaces,
        needs_helpers: needs_dart_helpers(snapshot),
    }
}

fn needs_dart_helpers(snapshot: &ContractSnapshot) -> bool {
    !snapshot.tables.is_empty()
        || !snapshot.routines.is_empty()
        || snapshot.types.values().any(|ty| {
            matches!(ty.kind, ContractTypeKind::Composite { .. } | ContractTypeKind::Enum { .. })
        })
}

#[cfg(test)]
mod tests {
    use kalamdb_sql::compile_contract_sql;

    use super::*;
    use crate::workflow::schema::{
        naming::{assign_names, NamingOptions},
        output::SchemaEmitInput,
        procedures::ProcedureCatalog,
    };

    #[test]
    fn generates_table_specs_and_row_codecs_from_sql() {
        let snapshot = compile_contract_sql(
            r#"
CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL,
  created_at TIMESTAMP
);
"#,
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
        let source = generate_dart_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains("Generated by kalam schema gen"));
        assert!(source.contains(&format!("contract_hash: {hash}")));
        assert!(!source.to_lowercase().contains("placeholder"));
        assert!(source.contains("import 'package:kalam_sync/kalam_sync.dart';"));
        assert!(source.contains("final class Users {"));
        assert!(source.contains("required this.id"));
        assert!(source.contains("required this.email"));
        assert!(source.contains("this.createdAt"));
        assert!(source.contains("factory Users.fromJson"));
        assert!(source.contains("static final users = KalamTableSpec<Users>("));
        assert!(source.contains("tableId: 'users'"));
        assert!(source.contains("keyColumn: 'id'"));
        assert!(source.contains("mode: KalamSyncMode.bidirectional"));
        assert!(source.contains("keyOf: (row) => row.kalamRowKey"));
        assert!(source.contains("encode: (row) => row.toJson()"));
        assert!(source.contains("decode: Users.fromJson"));
    }

    #[test]
    fn stream_tables_use_replica_only_mode() {
        let snapshot = compile_contract_sql(
            r#"
CREATE STREAM TABLE app.message_events (
  id TEXT PRIMARY KEY,
  payload JSON
);
"#,
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
        let source = generate_dart_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains("tableId: 'app.message_events'"));
        assert!(source.contains("final class AppMessageEvents {"));
        assert!(
            source.contains("static final appMessageEvents = KalamTableSpec<AppMessageEvents>(")
        );
        assert!(source.contains("mode: KalamSyncMode.replicaOnly"));
        assert!(source.contains("Map<String, Object?>? payload"));
    }

    #[test]
    fn nested_struct_and_alias_share_one_shape() {
        let snapshot = compile_contract_sql(
            r#"
CREATE SCHEMA chat;
CREATE TYPE chat.address AS (city TEXT, country TEXT);
CREATE TABLE chat.users (
  id BIGINT PRIMARY KEY,
  address chat.address,
  nickname TEXT
) ROW TYPE chat.user;
"#,
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
        let source = generate_dart_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains("final class ChatAddress {"));
        assert!(source.contains("final class ChatUser {"));
        assert!(source.contains("typedef ChatUsers = ChatUser;"));
        assert!(source.contains("ChatAddress? address"));
        assert!(source.contains("String? nickname"));
        assert!(source.contains("ChatAddress.fromJson"));
        assert!(!source.contains("final class ChatUsers {"));
    }

    #[test]
    fn empty_schema_emits_compilable_kalam_tables() {
        let snapshot = compile_contract_sql("-- no tables yet\n", "public").unwrap();
        let hash = kalamdb_sql::canonical_contract_hash(&snapshot);
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let source = generate_dart_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains("abstract final class KalamTables"));
        assert!(!source.contains("KalamTableSpec<"));
        assert!(!source.contains("static final"));
        assert!(!source.contains("final class KalamFunctions"));
    }

    #[test]
    fn write_dart_schema_creates_parent_directories() {
        let temp = tempfile::TempDir::new().unwrap();
        let output = temp.path().join("lib/generated/kalam.dart");
        let snapshot = compile_contract_sql(
            "CREATE TABLE todos (id TEXT PRIMARY KEY, title TEXT NOT NULL, done BOOLEAN NOT NULL);",
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
        write_dart_schema(&SchemaEmitInput {
            project_root: temp.path(),
            output_path:  &output,
            snapshot:     &snapshot,
            hash:         &hash,
            names:        &names,
            procedures:   &procedures,
        })
        .unwrap();
        let source = std::fs::read_to_string(&output).unwrap();
        assert!(source.contains("KalamTableSpec<Todos>"));
        assert!(source.contains("bool done"));
    }

    #[test]
    fn comments_emit_dart_docs() {
        let snapshot = compile_contract_sql(
            "CREATE SCHEMA chat;
             CREATE TYPE chat.address AS (city TEXT NOT NULL) COMMENT 'Postal address';",
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
        let source = generate_dart_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains("/// Postal address"), "{source}");
    }

    #[test]
    fn generates_typed_procedure_client() {
        let snapshot = compile_contract_sql(
            r#"
CREATE SCHEMA chat;
CREATE SCHEMA app;
CREATE TABLE chat.users (
  id BIGINT PRIMARY KEY,
  email TEXT NOT NULL
) ROW TYPE chat.user;
CREATE PROCEDURE chat.create_message(user_id TEXT NOT NULL, body TEXT NOT NULL)
RETURNS chat.user;
CREATE PROCEDURE chat.ping();
CREATE PROCEDURE app.send_message(body JSON NOT NULL) RETURNS JSON;
"#,
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
        let source = generate_dart_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains("final class ChatCreateMessageRequest"), "{source}");
        assert!(source.contains("required this.userId"), "{source}");
        assert!(source.contains("required this.body"), "{source}");
        let request = source
            .split("final class ChatCreateMessageRequest")
            .nth(1)
            .and_then(|rest| rest.split("typedef ChatCreateMessageResult").next())
            .unwrap_or("");
        assert!(!request.contains("kalamRowKey"), "{request}");
        assert!(source.contains("typedef ChatCreateMessageResult = ChatUser?;"), "{source}");
        assert!(source.contains("typedef ChatPingResult = void;"), "{source}");
        assert!(
            source.contains("typedef AppSendMessageResult = Map<String, Object?>?;"),
            "{source}"
        );
        assert!(source.contains("final class KalamFunctions"), "{source}");
        assert!(source.contains("late final chat = ChatFunctions(_client);"), "{source}");
        assert!(source.contains("late final app = AppFunctions(_client);"), "{source}");
        assert!(
            source.contains(
                "Future<ChatCreateMessageResult> createMessage(ChatCreateMessageRequest input)"
            ),
            "{source}"
        );
        assert!(source.contains("CALL chat.create_message($1, $2)"), "{source}");
        assert!(source.contains("input.userId, input.body"), "{source}");
        assert!(source.contains("final result = _kalamCallResult(response);"), "{source}");
        assert!(source.contains("ChatUser.fromJson(_asJsonMap(result))"), "{source}");
        assert!(source.contains("Future<ChatPingResult> ping() async"), "{source}");
        assert!(source.contains("CALL chat.ping()"), "{source}");
        assert!(source.contains("Future<AppSendMessageResult> sendMessage"), "{source}");
        assert!(source.contains("return _asJsonMapOrNull(result);"), "{source}");
        assert!(source.contains("Object? _kalamCallResult(QueryResponse response)"), "{source}");
        assert!(!source.contains("KalamClient.call"), "{source}");
        assert!(!source.contains("/v1/functions"), "{source}");
    }
}
