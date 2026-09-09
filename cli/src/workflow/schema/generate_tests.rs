//! Golden tests for local contract generation (F4).

use std::fs;

use kalamdb_sql::{canonical_contract_hash, compile_contract_sql};
use tempfile::TempDir;

use crate::workflow::{
    project::config::{KalamProjectConfig, SchemaMode, SchemaSection, SchemaTarget},
    schema::{
        dart::generate_dart_source,
        gen::generate_languages,
        naming::{assign_names, NamingOptions},
        rust::generate_rust_source,
        typescript::{
            generate_client_source, generate_contracts_source, generate_runtime_dts,
            generate_schema_source,
        },
        LanguageTarget,
    },
    test_support::minimal_sql_project_config,
};

const GOLDEN_SQL: &str = r#"
CREATE SCHEMA chat;
CREATE TYPE chat.address AS (city TEXT, country TEXT);
CREATE TYPE chat.status AS ENUM ('active', 'blocked');
CREATE TABLE chat.users (
  id BIGINT PRIMARY KEY,
  email TEXT NOT NULL,
  address chat.address,
  nickname TEXT,
  status chat.status NOT NULL
) ROW TYPE chat.user;
CREATE PROCEDURE chat.create_message(user_id TEXT, body TEXT NOT NULL)
RETURNS chat.user;
"#;

fn golden_snapshot() -> kalamdb_sql::contracts::ContractSnapshot {
    compile_contract_sql(GOLDEN_SQL, "public").expect("compile golden sql")
}

#[test]
fn all_targets_embed_the_same_contract_hash() {
    let snapshot = golden_snapshot();
    let hash = canonical_contract_hash(&snapshot);
    let names = assign_names(
        &snapshot,
        NamingOptions {
            unqualified_names: false,
        },
    )
    .unwrap();
    let ts = generate_client_source(&snapshot, &hash, &names);
    let schema = generate_schema_source(&snapshot, &hash, &names);
    let contracts = generate_contracts_source(&hash, "../../../src/generated/schema");
    let dart = generate_dart_source(&snapshot, &hash, &names);
    let rust = generate_rust_source(&snapshot, &hash, &names);
    let marker = format!("contract_hash: {hash}");
    assert!(ts.contains(&marker));
    assert!(schema.contains(&marker));
    assert!(contracts.contains(&marker));
    assert!(dart.contains(&marker));
    assert!(rust.contains(&marker));
    assert_eq!(hash.len(), 64);
}

#[test]
fn runtime_dts_nested_typed_call() {
    let snapshot = golden_snapshot();
    let names = assign_names(
        &snapshot,
        NamingOptions {
            unqualified_names: false,
        },
    )
    .unwrap();
    let dts = generate_runtime_dts(&snapshot, &names);
    assert!(dts.contains("createMessage(input: ChatCreateMessageRequest)"), "{dts}");
    assert!(dts.contains("Promise<ChatCreateMessageResult>"), "{dts}");
    assert!(dts.contains("chat:"), "{dts}");
    assert!(dts.contains("functions: FunctionsHost"), "{dts}");
    assert!(
        dts.contains(
            "import type { ChatCreateMessageRequest, ChatCreateMessageResult } from \
             \"./contracts\""
        ),
        "{dts}"
    );
}

#[test]
fn golden_nullability_nested_struct_alias_and_codecs() {
    let snapshot = golden_snapshot();
    let hash = canonical_contract_hash(&snapshot);
    let names = assign_names(
        &snapshot,
        NamingOptions {
            unqualified_names: false,
        },
    )
    .unwrap();
    let ts = generate_client_source(&snapshot, &hash, &names);
    let schema = generate_schema_source(&snapshot, &hash, &names);
    let dart = generate_dart_source(&snapshot, &hash, &names);
    let rust = generate_rust_source(&snapshot, &hash, &names);

    assert!(schema.contains("export type ChatAddress"));
    assert!(schema.contains("address: ChatAddress | null"));
    assert!(schema.contains("nickname: string | null"));
    assert!(schema.contains("email: string;"));
    assert!(schema.contains("export type ChatUser"));
    assert!(schema.contains("export type ChatUsers = ChatUser"));
    assert!(ts.contains("createMessage:"));
    assert!(ts.contains("chat: {"));
    assert!(ts.contains("export * from './schema'"));

    assert!(dart.contains("final class ChatAddress {"));
    assert!(dart.contains("ChatAddress? address"));
    assert!(dart.contains("String? nickname"));
    assert!(dart.contains("typedef ChatUsers = ChatUser;"));
    assert!(dart.contains("ChatAddress.fromJson"));

    assert!(rust.contains("pub struct ChatAddress"));
    assert!(rust.contains("pub address: Option<ChatAddress>"));
    assert!(rust.contains("pub nickname: Option<String>"));
    assert!(rust.contains("pub type ChatUsers = ChatUser;"));
}

#[test]
fn unqualified_names_emit_short_idents() {
    let snapshot = golden_snapshot();
    let hash = canonical_contract_hash(&snapshot);
    let names = assign_names(
        &snapshot,
        NamingOptions {
            unqualified_names: true,
        },
    )
    .unwrap();
    let ts = generate_client_source(&snapshot, &hash, &names);
    let schema = generate_schema_source(&snapshot, &hash, &names);
    assert!(schema.contains("export type Address"));
    assert!(schema.contains("export type User"));
    assert!(ts.contains("createMessage:"));
    assert!(ts.contains("chat: {"));
    assert!(!schema.contains("export type ChatUser"));
}

#[test]
fn scaffold_writes_once_and_refuses_missing_export() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("schema.sql"), GOLDEN_SQL).unwrap();
    let mut config = minimal_sql_project_config();
    config.schema = SchemaSection {
        mode:      SchemaMode::Sql,
        path:      Some("schema.sql".into()),
        watch:     false,
        languages: vec!["typescript".into()],
        targets:   [(
            "typescript".into(),
            SchemaTarget {
                output:            "src/generated/kalam.ts".into(),
                unqualified_names: false,
            },
        )]
        .into(),
    };

    generate_languages(root, &config, &[LanguageTarget::TypeScript], None).unwrap();

    let impl_path = root.join("functions/src/chat/create_message.ts");
    let original = fs::read_to_string(&impl_path).unwrap();
    assert!(original.contains("export default defineProcedure("));
    assert!(original.contains("type ProcedureContext"));
    assert!(original.contains("type ChatCreateMessageRequest"));
    assert!(original.contains("type ChatCreateMessageResult"));
    assert!(original.contains(
        "async (ctx: ProcedureContext, input: ChatCreateMessageRequest): \
         Promise<ChatCreateMessageResult>"
    ));
    assert!(original.contains("../generated/contracts"));
    fs::write(
        &impl_path,
        original.replace("throw new Error(\"not implemented\")", "return input as never"),
    )
    .unwrap();
    let generated = root.join("functions/src/generated");
    fs::create_dir_all(&generated).unwrap();
    fs::write(
        generated.join("runtime.ts"),
        "export function defineProcedure(handler) { return handler; }\n",
    )
    .unwrap();
    let stale_dot_kalam = root.join("functions/.kalam/generated");
    fs::create_dir_all(&stale_dot_kalam).unwrap();
    fs::write(stale_dot_kalam.join("contracts.ts"), "// stale\n").unwrap();

    generate_languages(root, &config, &[LanguageTarget::TypeScript], None).unwrap();
    let after = fs::read_to_string(&impl_path).unwrap();
    assert!(after.contains("return input as never"));
    assert!(!after.contains("not implemented"));

    let registry = fs::read_to_string(root.join("functions/src/generated/registry.ts")).unwrap();
    assert!(registry.contains("from \"../chat/create_message\""));
    assert!(registry.contains("\"chat.create_message\""));

    let runtime = fs::read_to_string(root.join("functions/src/generated/runtime.d.ts")).unwrap();
    assert!(runtime.contains("export interface ProcedureContext"));
    assert!(runtime.contains("readonly orm: ProcedureOrm"));
    assert!(runtime.contains("defineProcedure"));
    assert!(runtime.contains("ctx: ProcedureContext"));
    assert!(runtime.contains("input: TRequest"));
    assert!(runtime.contains("createMessage(input: ChatCreateMessageRequest)"));
    assert!(runtime.contains("chat:"));
    assert!(root.join("functions/src/generated/runtime.js").is_file());
    let runtime_js = fs::read_to_string(root.join("functions/src/generated/runtime.js")).unwrap();
    assert!(runtime_js.contains("bindFunctionOrm"));
    assert!(!root.join("functions/src/generated/runtime.ts").exists());
    assert!(!stale_dot_kalam.exists());
    let contracts = fs::read_to_string(root.join("functions/src/generated/contracts.ts")).unwrap();
    assert!(contracts.contains("from \"./runtime\""));
    assert!(contracts.contains("from \"../../../src/generated/schema\""));
    assert!(root.join("src/generated/schema.ts").is_file());
    let schema = fs::read_to_string(root.join("src/generated/schema.ts")).unwrap();
    assert!(schema.contains("export type ChatCreateMessageRequest"));
    assert!(schema.contains("export type ChatCreateMessageResult"));

    fs::write(&impl_path, "export const broken = 1;\n").unwrap();
    let err = generate_languages(root, &config, &[LanguageTarget::TypeScript], None).unwrap_err();
    assert!(err.to_string().contains("export default"), "{err}");
}

#[test]
fn inline_routines_skip_src_scaffold_and_get_typecheck_shim() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("schema.sql"),
        r#"
CREATE SCHEMA api;
CREATE PROCEDURE api.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return "ok"; $$;
"#,
    )
    .unwrap();
    let mut config = minimal_sql_project_config();
    config.schema = SchemaSection {
        mode:      SchemaMode::Sql,
        path:      Some("schema.sql".into()),
        watch:     false,
        languages: vec!["typescript".into()],
        targets:   [(
            "typescript".into(),
            SchemaTarget {
                output:            "src/generated/kalam.ts".into(),
                unqualified_names: false,
            },
        )]
        .into(),
    };
    generate_languages(root, &config, &[LanguageTarget::TypeScript], None).unwrap();
    assert!(!root.join("functions/src/api/health.ts").exists());
    let shim =
        fs::read_to_string(root.join("functions/src/generated/inline/api_health.ts")).unwrap();
    assert!(shim.contains("ProcedureContext"));
    assert!(shim.contains("return \"ok\""));
    let registry = fs::read_to_string(root.join("functions/src/generated/registry.ts")).unwrap();
    assert!(!registry.contains("api.health"));
}

#[test]
fn generate_does_not_require_a_server_url() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("schema.sql"),
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);",
    )
    .unwrap();
    let config = KalamProjectConfig {
        schema: SchemaSection {
            mode:      SchemaMode::Sql,
            path:      Some("schema.sql".into()),
            watch:     false,
            languages: vec!["typescript".into(), "dart".into(), "rust".into()],
            targets:   [
                (
                    "typescript".into(),
                    SchemaTarget {
                        output:            "src/generated/kalam.ts".into(),
                        unqualified_names: false,
                    },
                ),
                (
                    "dart".into(),
                    SchemaTarget {
                        output:            "lib/generated/kalam.dart".into(),
                        unqualified_names: false,
                    },
                ),
                (
                    "rust".into(),
                    SchemaTarget {
                        output:            "src/generated/kalam.rs".into(),
                        unqualified_names: false,
                    },
                ),
            ]
            .into(),
        },
        ..minimal_sql_project_config()
    };
    generate_languages(
        root,
        &config,
        &[
            LanguageTarget::TypeScript,
            LanguageTarget::Dart,
            LanguageTarget::Rust,
        ],
        None,
    )
    .unwrap();
    let ts = fs::read_to_string(root.join("src/generated/kalam.ts")).unwrap();
    let dart = fs::read_to_string(root.join("lib/generated/kalam.dart")).unwrap();
    let rust = fs::read_to_string(root.join("src/generated/kalam.rs")).unwrap();
    let hash_line = ts
        .lines()
        .find(|line| line.contains("contract_hash:"))
        .expect("ts hash")
        .to_string();
    assert!(dart.contains(&hash_line[3..]) || dart.contains(hash_line.trim_start_matches("// ")));
    assert!(rust.contains(hash_line.trim_start_matches("// ")));
    let schema = fs::read_to_string(root.join("src/generated/schema.ts")).unwrap();
    assert!(schema.contains("export const users"));
    assert!(dart.contains("KalamTableSpec<Users>"));
}
