//! Rust types generated from a [`ContractSnapshot`].

use kalamdb_sql::contracts::{ContractField, ContractSnapshot, ContractTypeKind};
use serde::Serialize;

use crate::{
    error::Result,
    workflow::{
        project::templates::render_schema_gen_file,
        schema::{
            naming::{pascal_case, AssignedNames},
            output::{
                canonical_row_type_id, line_docs, row_type_ident, type_comment, write_text,
                GeneratedHeader, SchemaEmitInput,
            },
            procedures::ProcedureCatalog,
            types::{render_field_type, TargetLang},
        },
    },
};

const RUST_TEMPLATE: &str = "src/generated/kalam.rs";

#[derive(Serialize)]
struct RustContext {
    #[serde(flatten)]
    file:  GeneratedHeader,
    decls: Vec<RustDecl>,
}

#[derive(Serialize, Default)]
struct RustDecl {
    doc:       String,
    ident:     String,
    is_enum:   bool,
    is_struct: bool,
    is_alias:  bool,
    is_topic:  bool,
    alias_of:  String,
    variants:  Vec<RustVariant>,
    fields:    Vec<RustField>,
}

#[derive(Serialize)]
struct RustVariant {
    ident: String,
}

#[derive(Serialize)]
struct RustField {
    name: String,
    ty:   String,
}

pub fn write_rust_schema(input: &SchemaEmitInput<'_>) -> Result<()> {
    write_text(
        input.output_path,
        &render_rust_source(input.snapshot, input.hash, input.names, input.procedures)?,
    )
}

pub fn generate_rust_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
) -> Result<String> {
    let procedures = ProcedureCatalog::from_snapshot(snapshot, names);
    render_rust_source(snapshot, hash, names, &procedures)
}

fn render_rust_source(
    snapshot: &ContractSnapshot,
    hash: &str,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> Result<String> {
    render_schema_gen_file(
        "rust",
        RUST_TEMPLATE,
        &RustContext {
            file:  GeneratedHeader::from_hash(hash),
            decls: rust_decls(snapshot, names, procedures),
        },
    )
}

fn rust_decls(
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> Vec<RustDecl> {
    let mut decls = Vec::new();
    for (id, ty) in &snapshot.types {
        match &ty.kind {
            ContractTypeKind::Enum { labels } => {
                decls.push(RustDecl {
                    doc: line_docs(ty.comment.as_deref(), "/// "),
                    ident: names.type_ident(id).to_string(),
                    is_enum: true,
                    variants: labels
                        .iter()
                        .map(|label| RustVariant {
                            ident: pascal_case(label),
                        })
                        .collect(),
                    ..RustDecl::default()
                });
            },
            ContractTypeKind::Composite { fields } => {
                decls.push(struct_decl(names.type_ident(id), fields, names, ty.comment.as_deref()));
            },
            ContractTypeKind::ImplicitTableRow { .. }
            | ContractTypeKind::RowAlias { .. }
            | ContractTypeKind::TopicPayload { .. } => {},
        }
    }

    for table in snapshot.tables.values() {
        let canonical_id = canonical_row_type_id(table).to_string();
        decls.push(struct_decl(
            names.type_ident(&canonical_id),
            &table.fields,
            names,
            type_comment(snapshot, &canonical_id)
                .or_else(|| type_comment(snapshot, table.row_type_id.as_str())),
        ));
        if let Some(alias) = &table.row_alias_id {
            let alias_ident = names.type_ident(alias.as_str());
            let row_ident = names.type_ident(table.row_type_id.as_str());
            if alias_ident != row_ident {
                decls.push(RustDecl {
                    ident: row_ident.to_string(),
                    is_alias: true,
                    alias_of: alias_ident.to_string(),
                    ..RustDecl::default()
                });
            }
        }
    }

    for (id, ty) in &snapshot.types {
        if let ContractTypeKind::TopicPayload { sources, .. } = &ty.kind {
            decls.push(RustDecl {
                ident: names.type_ident(id).to_string(),
                is_topic: true,
                variants: sources
                    .iter()
                    .map(|table_id| RustVariant {
                        ident: row_type_ident(snapshot, names, table_id).to_string(),
                    })
                    .collect(),
                ..RustDecl::default()
            });
        }
    }

    for spec in procedures.procedures() {
        decls.push(RustDecl {
            doc: line_docs(spec.comment(), "/// "),
            ident: spec.rust_input_ident(),
            is_struct: true,
            fields: spec.parameters().iter().map(|field| rust_field(field, names)).collect(),
            ..RustDecl::default()
        });
        decls.push(RustDecl {
            ident: spec.rust_output_ident(),
            is_alias: true,
            alias_of: spec
                .return_type()
                .map(|ret| render_field_type(ret, names, TargetLang::Rust))
                .unwrap_or_else(|| "()".to_string()),
            ..RustDecl::default()
        });
    }
    decls
}

fn struct_decl(
    ident: &str,
    fields: &[ContractField],
    names: &AssignedNames,
    comment: Option<&str>,
) -> RustDecl {
    RustDecl {
        doc: line_docs(comment, "/// "),
        ident: ident.to_string(),
        is_struct: true,
        fields: fields.iter().map(|field| rust_field(field, names)).collect(),
        ..RustDecl::default()
    }
}

fn rust_field(field: &ContractField, names: &AssignedNames) -> RustField {
    RustField {
        name: rust_field_name(&field.name),
        ty:   render_field_type(field, names, TargetLang::Rust),
    }
}

fn rust_field_name(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while",
    ];
    if KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_sql::compile_contract_sql;

    use super::*;
    use crate::workflow::schema::naming::{assign_names, NamingOptions};

    #[test]
    fn rust_reuses_alias_and_nests_struct_fields() {
        let snapshot = compile_contract_sql(
            "CREATE SCHEMA chat;
             CREATE TYPE chat.address AS (city TEXT, country TEXT);
             CREATE TABLE chat.users (
               id BIGINT PRIMARY KEY,
               address chat.address,
               nickname TEXT
             ) ROW TYPE chat.user;",
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
        let source = generate_rust_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains(&format!("contract_hash: {hash}")));
        assert!(source.contains("pub struct ChatAddress"));
        assert!(source.contains("pub struct ChatUser"));
        assert!(source.contains("pub type ChatUsers = ChatUser;"));
        assert!(source.contains("pub address: Option<ChatAddress>"));
        assert!(source.contains("pub nickname: Option<String>"));
        assert!(source.contains("pub id: i64"));
    }

    #[test]
    fn topic_payload_emits_enum_after_table_structs() {
        let snapshot = compile_contract_sql(
            r#"
CREATE SCHEMA chat;
CREATE TABLE chat.messages (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
CREATE TABLE chat.direct_messages (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
CREATE TOPIC chat.ai_inbox;
ALTER TOPIC chat.ai_inbox ADD SOURCE chat.messages ON INSERT;
ALTER TOPIC chat.ai_inbox ADD SOURCE chat.direct_messages ON INSERT;
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
        let source = generate_rust_source(&snapshot, &hash, &names).unwrap();
        let messages = source.find("pub struct ChatMessages").expect(source.as_str());
        let inbox = source.find("pub enum ChatAiInbox").expect(source.as_str());
        assert!(inbox > messages, "{source}");
        assert!(source.contains("ChatDirectMessages(ChatDirectMessages)"));
        assert!(source.contains("ChatMessages(ChatMessages)"));
    }

    #[test]
    fn comments_emit_rust_docs() {
        let snapshot = compile_contract_sql(
            "CREATE SCHEMA chat;
             CREATE TYPE chat.address AS (city TEXT NOT NULL) COMMENT 'Postal address';
             CREATE PROCEDURE chat.ping() RETURNS TEXT COMMENT 'Liveness probe';",
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
        let source = generate_rust_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains("/// Postal address"), "{source}");
        assert!(source.contains("/// Liveness probe"), "{source}");
    }
}
