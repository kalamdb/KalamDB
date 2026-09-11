//! TypeScript client, server contracts, and one-shot procedure scaffolds.

use crate::{
    error::Result,
    workflow::schema::output::{write_text, SchemaEmitInput},
};

mod client;
mod host;
mod schema;
mod text;

pub use client::generate_client_source;
pub use host::{
    generate_contracts_source, generate_registry_source, generate_runtime_dts, generate_runtime_js,
    implemented_procedure_source, procedure_impl_path,
};
pub use schema::generate_schema_source;

pub fn write_typescript(input: &SchemaEmitInput<'_>) -> Result<()> {
    let schema_path = input.output_path.with_file_name("schema.ts");
    write_text(
        &schema_path,
        &schema::render_schema_source(input.snapshot, input.hash, input.names, input.procedures)?,
    )?;
    write_text(input.output_path, &client::render_client_source(input.hash, input.procedures)?)?;
    host::write_procedure_artifacts(
        input.project_root,
        &schema_path,
        input.snapshot,
        input.hash,
        input.procedures,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use kalamdb_sql::compile_contract_sql;

    use super::*;
    use crate::workflow::schema::naming::{assign_names, NamingOptions};

    #[test]
    fn client_embeds_hash_nested_client_and_prefixed_types() {
        let snapshot = compile_contract_sql(GOLDEN_SQL, "public").unwrap();
        let hash = kalamdb_sql::canonical_contract_hash(&snapshot);
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let schema = generate_schema_source(&snapshot, &hash, &names).unwrap();
        let source = generate_client_source(&snapshot, &hash, &names).unwrap();
        assert!(source.contains(&format!("contract_hash: {hash}")));
        assert!(source.contains("export * from './schema'"));
        assert!(schema.contains("export type ChatAddress"));
        assert!(schema.contains("address: ChatAddress | null"));
        assert!(schema.contains("nickname: string | null"));
        assert!(schema.contains("export type ChatUser"));
        assert!(schema.contains("export type ChatUsers = ChatUser"));
        assert!(schema.contains("export type ChatStatus = \"active\" | \"blocked\""));
        assert!(source.contains("createKalam"));
        assert!(source.contains("CALL chat.create_message($1, $2)"));
        assert!(!source.contains("procedure client has no local runtime"));
        assert!(source.contains("chat: {"));
        assert!(source.contains("createMessage:"));
        assert!(schema.contains("export type ChatCreateMessageRequest"));
        assert!(schema.contains("export type ChatCreateMessageResult"));
        assert!(source.contains("createMessage: async (input: ChatCreateMessageRequest)"));
        assert!(!source.contains("kalam.chat.createMessage"));
        assert!(source.contains("rows?: unknown;"));
        assert!(source.contains("response.status === \"error\""));
        assert!(source.contains("throw new Error(kalamCallErrorMessage(response.error))"));
        assert!(source.contains("Array.isArray(first) ? first[0] : first"));
    }

    #[test]
    fn topic_payload_emits_tagged_union() {
        let snapshot = compile_contract_sql(
            r#"
CREATE SCHEMA chat;
CREATE TABLE chat.messages (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
CREATE TABLE chat.direct_messages (id BIGINT PRIMARY KEY, body TEXT NOT NULL);
CREATE TOPIC chat.ai_inbox;
ALTER TOPIC chat.ai_inbox ADD SOURCE chat.messages ON INSERT;
ALTER TOPIC chat.ai_inbox ADD SOURCE chat.direct_messages ON INSERT;
CREATE PROCEDURE chat.on_message(payload chat.ai_inbox NOT NULL)
RETURNS chat.ai_inbox;
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
        let schema = generate_schema_source(&snapshot, &hash, &names).unwrap();
        assert!(schema.contains("export type ChatAiInbox ="), "{schema}");
        assert!(
            schema.contains("({ _table: \"chat:direct_messages\" } & ChatDirectMessages)"),
            "{schema}"
        );
        assert!(schema.contains("({ _table: \"chat:messages\" } & ChatMessages)"), "{schema}");
        assert!(schema.contains("payload: ChatAiInbox;"), "{schema}");
        assert!(
            schema.contains("export type ChatOnMessageResult = ChatAiInbox | null;"),
            "{schema}"
        );
    }

    #[test]
    fn comments_emit_jsdoc() {
        let snapshot = compile_contract_sql(
            r#"
CREATE SCHEMA chat;
CREATE TYPE chat.address AS (city TEXT NOT NULL) COMMENT 'Postal address';
CREATE PROCEDURE chat.ping() RETURNS TEXT COMMENT 'Liveness probe';
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
        let schema = generate_schema_source(&snapshot, &hash, &names).unwrap();
        let client = generate_client_source(&snapshot, &hash, &names).unwrap();
        assert!(schema.contains("/** Postal address */"), "{schema}");
        assert!(schema.contains("/** Liveness probe */"), "{schema}");
        assert!(client.contains("/** Liveness probe */"), "{client}");
    }

    #[test]
    fn runtime_js_binds_ctx_orm() {
        let js = generate_runtime_js().unwrap();
        assert!(js.contains("export function wrapProcedure(handler)"));
        assert!(js.contains("import { bindFunctionOrm } from \"@kalamdb/orm\""));
        assert!(js.contains("bindFunctionOrm(ctx.db)"));
        assert!(js.contains("next, \"orm\""));
    }

    #[test]
    fn runtime_dts_lists_ctx_abi() {
        let snapshot = compile_contract_sql("-- no procedures\n", "public").unwrap();
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let dts = generate_runtime_dts(&snapshot, &names, "./schema").unwrap();
        assert!(dts.contains("export interface ProcedureContext"), "{dts}");
        assert!(dts.contains("import type { ProcedureOrm } from \"@kalamdb/orm\""), "{dts}");
        assert!(dts.contains("readonly orm: ProcedureOrm"), "{dts}");
        assert!(dts.contains("sleep(ms: number): Promise<void>"), "{dts}");
        assert!(
            dts.contains("error(error: unknown, message?: string, ...args: unknown[]): void"),
            "{dts}"
        );
        assert!(!dts.contains("defineProcedure"), "{dts}");
        assert!(dts.contains("channel=console"), "{dts}");
        assert!(dts.contains("call(name: string, args?: unknown): Promise<unknown>"), "{dts}");
        assert!(!dts.contains("from \"./schema\""), "{dts}");
    }

    #[test]
    fn procedure_builders_use_schema_types() {
        let snapshot = compile_contract_sql(GOLDEN_SQL, "public").unwrap();
        let hash = kalamdb_sql::canonical_contract_hash(&snapshot);
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let output = temp.path().join("src/generated/kalam.ts");
        let procedures =
            crate::workflow::schema::procedures::ProcedureCatalog::from_snapshot(&snapshot, &names);
        write_typescript(&crate::workflow::schema::output::SchemaEmitInput {
            project_root: temp.path(),
            output_path:  &output,
            snapshot:     &snapshot,
            hash:         &hash,
            names:        &names,
            procedures:   &procedures,
        })
        .unwrap();
        let dts =
            std::fs::read_to_string(temp.path().join("functions/src/generated/procedure.d.ts"))
                .unwrap();
        let js = std::fs::read_to_string(temp.path().join("functions/src/generated/procedure.js"))
            .unwrap();
        assert!(
            dts.contains("import type { ChatCreateMessageRequest, ChatCreateMessageResult } from ")
        );
        assert!(dts.contains(
            "createMessage: ProcedureBuilder<ChatCreateMessageRequest, ChatCreateMessageResult>"
        ));
        assert!(dts.contains("chat:"));
        assert!(dts.contains("ctx: ProcedureContext"));
        assert!(dts.contains("input: TInput"));
        assert!(js.contains("import { wrapProcedure } from \"./runtime.js\""));
        assert!(js.contains("createMessage: procedureBuilder()"));
        assert!(js.contains("unimplemented"));
    }

    #[test]
    fn empty_arg_emits_no_input_param() {
        let snapshot =
            compile_contract_sql("CREATE SCHEMA api; CREATE PROCEDURE api.health();", "public")
                .unwrap();
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let dts = generate_runtime_dts(&snapshot, &names, "./schema").unwrap();
        assert!(dts.contains("health(): Promise<ApiHealthResult>"), "{dts}");
        assert!(!dts.contains("health(input:"), "{dts}");
        assert!(dts.contains("import type { ApiHealthResult } from \"./schema\""), "{dts}");
    }

    #[test]
    fn contracts_import_schema_relatively() {
        let from = PathBuf::from("/proj/functions/src/generated");
        let target = PathBuf::from("/proj/src/generated/schema.ts");
        assert_eq!(text::ts_relative_module(&from, &target), "../../../src/generated/schema");
    }

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
}
