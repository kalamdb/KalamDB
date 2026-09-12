//! Shared generated identifiers for TypeScript, Dart, and Rust.

use std::collections::BTreeMap;

pub use kalamdb_functions_host::{
    camel_case, method_ident, namespace_object_ident, pascal_case, DEFAULT_NAMESPACE,
};
use kalamdb_sql::contracts::ContractSnapshot;

use crate::error::{CLIError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamingOptions {
    pub unqualified_names: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedNames {
    /// Qualified type id (`chat.user`) → generated type ident (`ChatUser` / `User`).
    pub types:    BTreeMap<String, String>,
    /// Qualified routine id → generated procedure contract ident.
    pub routines: BTreeMap<String, String>,
}

impl AssignedNames {
    pub fn type_ident(&self, type_id: &str) -> &str {
        self.types.get(type_id).map(String::as_str).unwrap_or("Unknown")
    }

    pub fn routine_ident(&self, routine_id: &str) -> &str {
        self.routines.get(routine_id).map(String::as_str).unwrap_or("Unknown")
    }
}

pub fn assign_names(snapshot: &ContractSnapshot, options: NamingOptions) -> Result<AssignedNames> {
    let mut claimed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut types = BTreeMap::new();
    for (id, ty) in &snapshot.types {
        let ident = generated_type_ident(&ty.schema, &ty.name, options.unqualified_names);
        claimed.entry(ident.clone()).or_default().push(id.clone());
        types.insert(id.clone(), ident);
    }
    let mut routines = BTreeMap::new();
    for (id, routine) in &snapshot.routines {
        let ident = generated_type_ident(&routine.schema, &routine.name, options.unqualified_names);
        claimed.entry(ident.clone()).or_default().push(format!("procedure {id}"));
        routines.insert(id.clone(), ident);
    }

    let collisions: Vec<String> = claimed
        .into_iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(ident, owners)| format!("'{ident}' <- {}", owners.join(", ")))
        .collect();
    if !collisions.is_empty() {
        return Err(CLIError::ConfigurationError(format!(
            "generated type name collision (call paths stay nested; names are not flattened): {}. \
             Set unqualified_names = false or rename one of the SQL objects",
            collisions.join("; ")
        )));
    }

    Ok(AssignedNames { types, routines })
}

pub fn generated_type_ident(schema: &str, name: &str, unqualified_names: bool) -> String {
    let local = pascal_case(name);
    if unqualified_names || schema.eq_ignore_ascii_case(DEFAULT_NAMESPACE) {
        local
    } else {
        format!("{}{local}", pascal_case(schema))
    }
}

pub fn value_ident(schema: &str, name: &str, unqualified_names: bool) -> String {
    let local = camel_case(name);
    if unqualified_names || schema.eq_ignore_ascii_case(DEFAULT_NAMESPACE) {
        local
    } else {
        format!("{}{}", camel_case(schema), pascal_case(name))
    }
}

pub fn contract_hash_line(hash: &str) -> String {
    format!("contract_hash: {hash}")
}

#[cfg(test)]
mod tests {
    use kalamdb_sql::compile_contract_sql;

    use super::*;

    #[test]
    fn schema_prefixed_names_keep_nested_identity() {
        let snapshot = compile_contract_sql(
            "CREATE SCHEMA chat; CREATE TYPE chat.user AS (id TEXT); CREATE PROCEDURE \
             chat.create_message(body TEXT);",
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
        assert_eq!(names.type_ident("chat.user"), "ChatUser");
        assert_eq!(names.routine_ident("chat.create_message"), "ChatCreateMessage");
        assert_eq!(namespace_object_ident("chat"), "chat");
        assert_eq!(method_ident("create_message"), "createMessage");
    }

    #[test]
    fn unqualified_names_fail_on_collision_without_flattening_paths() {
        let snapshot = compile_contract_sql(
            "CREATE SCHEMA chat; CREATE SCHEMA app;
             CREATE TYPE chat.user AS (id TEXT);
             CREATE TYPE app.user AS (id TEXT);",
            "public",
        )
        .unwrap();
        let err = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: true,
            },
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("collision"), "{message}");
        assert!(message.contains("chat.user"), "{message}");
        assert!(message.contains("app.user"), "{message}");
        assert!(!message.contains("kalam.chat"), "{message}");
    }
}
