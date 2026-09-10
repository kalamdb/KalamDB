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
    let dts = generate_runtime_dts(&snapshot, &names, "./schema");
    assert!(dts.contains("createMessage(input: ChatCreateMessageRequest)"), "{dts}");
    assert!(dts.contains("Promise<ChatCreateMessageResult>"), "{dts}");
    assert!(dts.contains("chat:"), "{dts}");
    assert!(dts.contains("functions: FunctionsHost"), "{dts}");
    assert!(
        dts.contains(
            "import type { ChatCreateMessageRequest, ChatCreateMessageResult } from \"./schema\""
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
fn scaffold_writes_once_and_does_not_recreate_moved_handlers() {
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
    assert!(original.contains("procedure.chat.createMessage.unimplemented()"));
    assert!(original.contains("export const createMessage"));
    assert!(original.contains("../generated/contracts"));
    fs::write(
        &impl_path,
        original.replace(
            "procedure.chat.createMessage.unimplemented();",
            "procedure.chat.createMessage(async (ctx, input) => input);",
        ),
    )
    .unwrap();
    let generated = root.join("functions/src/generated");
    fs::create_dir_all(&generated).unwrap();
    fs::write(
        generated.join("runtime.ts"),
        "export function wrapProcedure(handler) { return handler; }\n",
    )
    .unwrap();
    fs::write(generated.join("procedure.ts"), "export const procedure = {};\n").unwrap();
    let stale_dot_kalam = root.join("functions/.kalam/generated");
    fs::create_dir_all(&stale_dot_kalam).unwrap();
    fs::write(stale_dot_kalam.join("contracts.ts"), "// stale\n").unwrap();

    generate_languages(root, &config, &[LanguageTarget::TypeScript], None).unwrap();
    let after = fs::read_to_string(&impl_path).unwrap();
    assert!(after.contains("async (ctx, input) => input"));
    assert!(!after.contains(".unimplemented()"));

    let registry = fs::read_to_string(root.join("functions/src/generated/registry.ts")).unwrap();
    assert!(registry.contains("from \"../chat/create_message\""));
    assert!(registry.contains("\"chat.create_message\""));
    assert!(registry.contains("createMessage as chatCreateMessage"));

    let runtime = fs::read_to_string(root.join("functions/src/generated/runtime.d.ts")).unwrap();
    assert!(runtime.contains("export interface ProcedureContext"));
    assert!(runtime.contains("readonly orm: ProcedureOrm"));
    assert!(!runtime.contains("defineProcedure"));
    assert!(runtime.contains("createMessage(input: ChatCreateMessageRequest)"));
    assert!(runtime.contains("chat:"));
    assert!(runtime.contains("from \"../../../src/generated/schema\""));
    assert!(root.join("functions/src/generated/runtime.js").is_file());
    let runtime_js = fs::read_to_string(root.join("functions/src/generated/runtime.js")).unwrap();
    assert!(runtime_js.contains("bindFunctionOrm"));
    assert!(runtime_js.contains("wrapProcedure"));
    assert!(!root.join("functions/src/generated/runtime.ts").exists());
    assert!(!root.join("functions/src/generated/procedure.ts").exists());
    assert!(!stale_dot_kalam.exists());
    let procedure_dts =
        fs::read_to_string(root.join("functions/src/generated/procedure.d.ts")).unwrap();
    assert!(procedure_dts
        .contains("ProcedureBuilder<ChatCreateMessageRequest, ChatCreateMessageResult>"));
    assert!(procedure_dts.contains("ctx: ProcedureContext"));
    assert!(procedure_dts.contains("input: TInput"));
    assert!(procedure_dts.contains("from \"../../../src/generated/schema\""));
    let contracts = fs::read_to_string(root.join("functions/src/generated/contracts.ts")).unwrap();
    assert!(contracts.contains("from \"./runtime\""));
    assert!(contracts.contains("export { procedure } from \"./procedure.js\""));
    assert!(contracts.contains("from \"../../../src/generated/schema\""));
    assert!(root.join("src/generated/schema.ts").is_file());
    let schema = fs::read_to_string(root.join("src/generated/schema.ts")).unwrap();
    assert!(schema.contains("export type ChatCreateMessageRequest"));
    assert!(schema.contains("export type ChatCreateMessageResult"));

    fs::write(
        &root.join("functions/src/chat/handlers.ts"),
        "import { procedure } from \"../generated/contracts\";\nexport const createMessage = \
         procedure.chat.createMessage(async (ctx, input) => input);\n",
    )
    .unwrap();
    fs::remove_file(&impl_path).unwrap();
    generate_languages(root, &config, &[LanguageTarget::TypeScript], None).unwrap();
    assert!(!impl_path.exists(), "moved handler must not be re-scaffolded");
    let registry = fs::read_to_string(root.join("functions/src/generated/registry.ts")).unwrap();
    assert!(registry.contains("from \"../chat/handlers\""));
    assert!(registry.contains("\"chat.create_message\""));

    fs::write(&root.join("functions/src/chat/orphan.ts"), "export const broken = 1;\n").unwrap();
    generate_languages(root, &config, &[LanguageTarget::TypeScript], None).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("functions/src/chat/orphan.ts")).unwrap(),
        "export const broken = 1;\n"
    );
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

const TYPECHECK_SQL: &str = r#"
CREATE SCHEMA chat;
CREATE TYPE chat.user AS (id BIGINT, email TEXT NOT NULL, nickname TEXT);
CREATE PROCEDURE chat.create_message(user_id TEXT, body TEXT NOT NULL) RETURNS chat.user;
CREATE PROCEDURE chat.ping();
"#;

fn typescript_project_config() -> KalamProjectConfig {
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
    config
}

fn tsc_bin() -> Option<std::path::PathBuf> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        root.join("../examples/chat-with-ai/node_modules/.bin/tsc"),
        root.join("../node_modules/.bin/tsc"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn write_functions_tsconfig(root: &std::path::Path) {
    let functions = root.join("functions");
    fs::create_dir_all(functions.join("node_modules/@kalamdb/orm")).unwrap();
    fs::write(
        functions.join("node_modules/@kalamdb/orm/index.d.ts"),
        "export interface ProcedureOrm {}\n",
    )
    .unwrap();
    fs::write(
        functions.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "isolatedModules": true
  },
  "include": ["src"]
}
"#,
    )
    .unwrap();
}

fn run_tsc(root: &std::path::Path) -> Option<(bool, String)> {
    let tsc = tsc_bin()?;
    write_functions_tsconfig(root);
    let output = std::process::Command::new(tsc)
        .arg("--noEmit")
        .arg("-p")
        .arg("tsconfig.json")
        .current_dir(root.join("functions"))
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Some((output.status.success(), format!("{stdout}{stderr}")))
}

#[test]
fn named_builders_share_a_file_and_omit_unimplemented() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("schema.sql"),
        r#"
CREATE SCHEMA chat;
CREATE PROCEDURE chat.send_message(body TEXT NOT NULL) RETURNS TEXT;
CREATE PROCEDURE chat.join_room(room TEXT NOT NULL) RETURNS TEXT;
CREATE PROCEDURE chat.plus_one(x INT) RETURNS INT;
CREATE PROCEDURE chat.greet() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return "inline"; $$;
"#,
    )
    .unwrap();
    generate_languages(root, &typescript_project_config(), &[LanguageTarget::TypeScript], None)
        .unwrap();
    assert!(root.join("functions/src/chat/send_message.ts").is_file());
    assert!(root.join("functions/src/chat/join_room.ts").is_file());
    assert!(root.join("functions/src/chat/plus_one.ts").is_file());
    assert!(!root.join("functions/src/chat/handlers.ts").exists());
    fs::write(
        root.join("functions/src/chat/handlers.ts"),
        r#"
import { procedure } from "../generated/contracts";
export const send = procedure.chat.sendMessage(async (ctx, input) => input.body);
export const join = procedure.chat.joinRoom(async (_ctx, input) => input.room);
export function helper() { return 1; }
"#,
    )
    .unwrap();
    fs::remove_file(root.join("functions/src/chat/send_message.ts")).unwrap();
    fs::remove_file(root.join("functions/src/chat/join_room.ts")).unwrap();
    generate_languages(root, &typescript_project_config(), &[LanguageTarget::TypeScript], None)
        .unwrap();
    assert!(!root.join("functions/src/chat/send_message.ts").exists());
    assert!(!root.join("functions/src/chat/join_room.ts").exists());
    let plus_one = fs::read_to_string(root.join("functions/src/chat/plus_one.ts")).unwrap();
    assert!(plus_one.contains(".unimplemented()"));
    assert!(!root.join("functions/src/chat/greet.ts").exists());
    let registry = fs::read_to_string(root.join("functions/src/generated/registry.ts")).unwrap();
    assert!(registry.contains("from \"../chat/handlers\""));
    assert!(registry.contains("send as chatSendMessage"));
    assert!(registry.contains("join as chatJoinRoom"));
    assert!(registry.contains("\"chat.send_message\""));
    assert!(registry.contains("\"chat.join_room\""));
    assert!(!registry.contains("plus_one"));
    assert!(!registry.contains("greet"));
}

#[test]
fn procedure_builder_types_are_checked_by_tsc() {
    let Some(_) = tsc_bin() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("schema.sql"), TYPECHECK_SQL).unwrap();
    generate_languages(root, &typescript_project_config(), &[LanguageTarget::TypeScript], None)
        .unwrap();
    let generated = root.join("functions/src/generated");
    assert!(generated.join("procedure.js").is_file());
    assert!(generated.join("procedure.d.ts").is_file());
    assert!(!generated.join("procedure.ts").exists());
    assert!(generated.join("runtime.js").is_file());
    assert!(generated.join("runtime.d.ts").is_file());
    assert_eq!(
        generated.join("procedure.js").with_extension("d.ts").file_name().unwrap(),
        "procedure.d.ts"
    );

    fs::write(
        root.join("functions/src/chat/handlers.ts"),
        r#"
import { procedure } from "../generated/contracts";

export const createMessage = procedure.chat.createMessage(async (ctx, input) => {
  ctx.log.info(input.body);
  return { id: 1n, email: input.body, nickname: null };
});

export const ping = procedure.chat.ping(async () => {});
"#,
    )
    .unwrap();
    fs::remove_file(root.join("functions/src/chat/create_message.ts")).ok();
    fs::remove_file(root.join("functions/src/chat/ping.ts")).ok();

    let (ok, output) = run_tsc(root).expect("tsc");
    assert!(ok, "valid inferred handlers should typecheck: {output}");

    fs::write(
        root.join("functions/src/chat/handlers.ts"),
        r#"
import { procedure } from "../generated/contracts";
export const createMessage = procedure.chat.createMessage(async (_ctx, input) => {
  return input.missing;
});
"#,
    )
    .unwrap();
    let (ok, output) = run_tsc(root).expect("tsc");
    assert!(!ok, "invalid input access should fail tsc");
    assert!(
        output.contains("missing") || output.contains("Property"),
        "expected missing-property diagnostic: {output}"
    );

    fs::write(
        root.join("functions/src/chat/handlers.ts"),
        r#"
import { procedure } from "../generated/contracts";
export const createMessage = procedure.chat.createMessage(async () => 1);
"#,
    )
    .unwrap();
    let (ok, output) = run_tsc(root).expect("tsc");
    assert!(!ok, "wrong output type should fail tsc: {output}");

    fs::write(
        root.join("functions/src/chat/handlers.ts"),
        r#"
import { procedure } from "../generated/contracts";
export const createMessage = procedure.chat.createMessage(async () => ({ id: 1n }));
"#,
    )
    .unwrap();
    let (ok, output) = run_tsc(root).expect("tsc");
    assert!(!ok, "missing required return fields should fail tsc: {output}");
}
