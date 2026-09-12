//! Language-neutral procedure catalog for schema generation.
//!
//! TypeScript, Dart, and Rust adapters emit syntax from this catalog. CALL SQL,
//! namespace grouping, and contract type idents live here so a fourth language
//! does not copy those rules.

use std::collections::BTreeMap;

use kalamdb_sql::contracts::{ContractField, ContractRoutine, ContractSnapshot};

use crate::workflow::schema::naming::{method_ident, namespace_object_ident, AssignedNames};

#[derive(Debug, Clone)]
pub struct ProcedureCatalog<'a> {
    procedures: Vec<ProcedureSpec<'a>>,
    namespaces: Vec<ProcedureNamespace<'a>>,
}

#[derive(Debug, Clone)]
pub struct ProcedureNamespace<'a> {
    pub schema:       &'a str,
    pub object_ident: String,
    pub procedures:   Vec<ProcedureSpec<'a>>,
}

#[derive(Debug, Clone)]
pub struct ProcedureSpec<'a> {
    pub routine:      &'a ContractRoutine,
    pub type_ident:   &'a str,
    pub method_ident: String,
    pub call_sql:     String,
}

impl<'a> ProcedureCatalog<'a> {
    pub fn from_snapshot(snapshot: &'a ContractSnapshot, names: &'a AssignedNames) -> Self {
        let procedures: Vec<ProcedureSpec<'a>> = snapshot
            .routines
            .values()
            .map(|routine| ProcedureSpec::from_routine(routine, names))
            .collect();

        let mut grouped: BTreeMap<&str, Vec<ProcedureSpec<'a>>> = BTreeMap::new();
        for spec in &procedures {
            grouped.entry(spec.routine.schema.as_str()).or_default().push(spec.clone());
        }

        let namespaces = grouped
            .into_iter()
            .map(|(schema, specs)| ProcedureNamespace {
                schema,
                object_ident: namespace_object_ident(schema),
                procedures: specs,
            })
            .collect();

        Self {
            procedures,
            namespaces,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.procedures.is_empty()
    }

    pub fn procedures(&self) -> &[ProcedureSpec<'a>] {
        &self.procedures
    }

    pub fn namespaces(&self) -> &[ProcedureNamespace<'a>] {
        &self.namespaces
    }

    pub fn schema_type_imports(&self, include_empty_request: bool) -> String {
        let mut imports = Vec::with_capacity(self.procedures.len() * 2);
        for spec in &self.procedures {
            if include_empty_request || spec.has_params() {
                imports.push(spec.request_ident());
            }
            imports.push(spec.result_ident());
        }
        imports.sort();
        imports.dedup();
        imports.join(", ")
    }
}

impl<'a> ProcedureSpec<'a> {
    fn from_routine(routine: &'a ContractRoutine, names: &'a AssignedNames) -> Self {
        Self {
            routine,
            type_ident: names.routine_ident(routine.routine_id.as_str()),
            method_ident: method_ident(&routine.name),
            call_sql: call_sql(&routine.schema, &routine.name, routine.parameters.len()),
        }
    }

    pub fn request_ident(&self) -> String {
        format!("{}Request", self.type_ident)
    }

    pub fn result_ident(&self) -> String {
        format!("{}Result", self.type_ident)
    }

    pub fn rust_input_ident(&self) -> String {
        format!("{}Input", self.type_ident)
    }

    pub fn rust_output_ident(&self) -> String {
        format!("{}Output", self.type_ident)
    }

    pub fn has_params(&self) -> bool {
        !self.routine.parameters.is_empty()
    }

    pub fn parameters(&self) -> &[ContractField] {
        &self.routine.parameters
    }

    pub fn return_type(&self) -> Option<&ContractField> {
        self.routine.return_type.as_ref()
    }

    pub fn comment(&self) -> Option<&str> {
        self.routine.comment.as_deref()
    }
}

fn call_sql(schema: &str, name: &str, param_count: usize) -> String {
    if param_count == 0 {
        return format!("CALL {schema}.{name}()");
    }
    let placeholders = (1..=param_count).map(|index| format!("${index}")).collect::<Vec<_>>();
    format!("CALL {schema}.{name}({})", placeholders.join(", "))
}

#[cfg(test)]
mod tests {
    use kalamdb_sql::compile_contract_sql;

    use super::*;
    use crate::workflow::schema::naming::{assign_names, NamingOptions};

    #[test]
    fn call_sql_emits_postgres_placeholders() {
        assert_eq!(call_sql("chat", "ping", 0), "CALL chat.ping()");
        assert_eq!(call_sql("chat", "echo", 1), "CALL chat.echo($1)");
        assert_eq!(call_sql("chat", "create_message", 2), "CALL chat.create_message($1, $2)");
    }

    #[test]
    fn catalog_groups_by_schema_and_preserves_contract_idents() {
        let snapshot = compile_contract_sql(
            r#"
CREATE SCHEMA chat;
CREATE SCHEMA app;
CREATE PROCEDURE chat.create_message(user_id TEXT, body TEXT NOT NULL) RETURNS TEXT;
CREATE PROCEDURE chat.ping();
CREATE PROCEDURE app.send_message(body JSON NOT NULL) RETURNS JSON;
"#,
            "public",
        )
        .unwrap();
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        let catalog = ProcedureCatalog::from_snapshot(&snapshot, &names);

        let ids: Vec<&str> = catalog
            .procedures()
            .iter()
            .map(|spec| spec.routine.routine_id.as_str())
            .collect();
        assert_eq!(ids, ["app.send_message", "chat.create_message", "chat.ping"]);

        let namespaces: Vec<&str> = catalog.namespaces().iter().map(|ns| ns.schema).collect();
        assert_eq!(namespaces, ["app", "chat"]);
        assert_eq!(catalog.namespaces()[0].object_ident, "app");
        assert_eq!(catalog.namespaces()[1].object_ident, "chat");

        let create = &catalog.procedures()[1];
        assert_eq!(create.method_ident, "createMessage");
        assert_eq!(create.request_ident(), "ChatCreateMessageRequest");
        assert_eq!(create.result_ident(), "ChatCreateMessageResult");
        assert_eq!(create.rust_input_ident(), "ChatCreateMessageInput");
        assert_eq!(create.call_sql, "CALL chat.create_message($1, $2)");
        assert_eq!(catalog.procedures().len(), 3);
        assert_eq!(
            catalog.schema_type_imports(false),
            "AppSendMessageRequest, AppSendMessageResult, ChatCreateMessageRequest, \
             ChatCreateMessageResult, ChatPingResult"
        );
        assert_eq!(
            catalog.schema_type_imports(true),
            "AppSendMessageRequest, AppSendMessageResult, ChatCreateMessageRequest, \
             ChatCreateMessageResult, ChatPingRequest, ChatPingResult"
        );
    }
}
