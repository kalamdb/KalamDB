//! Shared generated identifiers for TypeScript, Dart, and Rust.

use std::collections::BTreeMap;

pub use kalamdb_functions_host::{
    camel_case, method_ident, namespace_object_ident, pascal_case, DEFAULT_NAMESPACE,
};
use kalamdb_sql::contracts::{
    scoped_contract_hashes, ContractRoutine, ContractSnapshot, ScopedContractHashes,
    CURRENT_CONTRACT_EPOCH,
};
use serde::{Deserialize, Serialize};

use crate::error::{CLIError, Result};

/// Current-epoch manifest recording deterministic SQL-to-public name mappings and scoped hashes
/// (`T004`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentEpochManifest {
    pub epoch: u16,
    pub scoped_hashes: ScopedContractHashes,
    pub sql_to_public_types: BTreeMap<String, String>,
    pub sql_to_public_routines: BTreeMap<String, String>,
}

impl CurrentEpochManifest {
    pub fn from_snapshot(snapshot: &ContractSnapshot, names: &AssignedNames) -> Self {
        Self {
            epoch: CURRENT_CONTRACT_EPOCH,
            scoped_hashes: scoped_contract_hashes(snapshot),
            sql_to_public_types: names.types.clone(),
            sql_to_public_routines: names.routines.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamingOptions {
    pub unqualified_names: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedNames {
    /// Qualified type id (`chat.user`) → generated type ident (`ChatUser` / `User`).
    pub types: BTreeMap<String, String>,
    /// Qualified routine id → generated procedure contract ident.
    pub routines: BTreeMap<String, String>,
    /// Project connection namespace. Names in this schema stay short.
    pub home_schema: Option<String>,
}

impl AssignedNames {
    pub fn type_ident(&self, type_id: &str) -> &str {
        self.types.get(type_id).map(String::as_str).unwrap_or("never")
    }

    pub fn routine_ident(&self, routine_id: &str) -> &str {
        self.routines.get(routine_id).map(String::as_str).unwrap_or("Unknown")
    }

    /// True when generated idents for `schema` should drop the schema prefix.
    pub fn uses_local_name(&self, schema: &str) -> bool {
        schema.eq_ignore_ascii_case(DEFAULT_NAMESPACE)
            || self
                .home_schema
                .as_deref()
                .is_some_and(|home| schema.eq_ignore_ascii_case(home))
    }
}

pub fn assign_names(snapshot: &ContractSnapshot, options: NamingOptions) -> Result<AssignedNames> {
    assign_names_with_home(snapshot, options, None)
}

pub fn assign_names_with_home(
    snapshot: &ContractSnapshot,
    options: NamingOptions,
    home_schema: Option<&str>,
) -> Result<AssignedNames> {
    let mut claimed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut types = BTreeMap::new();
    for (id, ty) in &snapshot.types {
        let ident = generated_type_ident(
            &ty.schema,
            &ty.name,
            options.unqualified_names || schema_is_home(&ty.schema, home_schema),
        );
        claimed.entry(ident.clone()).or_default().push(id.clone());
        types.insert(id.clone(), ident.clone());
        types.insert(ty.type_id.to_string(), ident);
    }
    let mut routines = BTreeMap::new();
    let mut overload_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for routine in snapshot.routines.values() {
        *overload_counts
            .entry((routine.schema.clone(), routine.name.clone()))
            .or_insert(0) += 1;
    }
    for (id, routine) in &snapshot.routines {
        let is_overloaded = overload_counts
            .get(&(routine.schema.clone(), routine.name.clone()))
            .copied()
            .unwrap_or(0)
            > 1;
        let base_ident = generated_type_ident(
            &routine.schema,
            &routine.name,
            options.unqualified_names || schema_is_home(&routine.schema, home_schema),
        );
        let ident = if is_overloaded {
            format!("{base_ident}{}", overload_suffix(routine))
        } else {
            base_ident
        };
        claimed.entry(ident.clone()).or_default().push(format!("procedure {id}"));
        routines.insert(id.clone(), ident.clone());
        if !routine.signature.is_empty() && routine.signature != *id {
            routines.insert(routine.signature.clone(), ident);
        }
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

    Ok(AssignedNames {
        types,
        routines,
        home_schema: home_schema.map(str::to_string),
    })
}

fn schema_is_home(schema: &str, home_schema: Option<&str>) -> bool {
    home_schema.is_some_and(|home| schema.eq_ignore_ascii_case(home))
}

pub fn generated_type_ident(schema: &str, name: &str, unqualified_names: bool) -> String {
    let local = pascal_case(name);
    if unqualified_names
        || schema.eq_ignore_ascii_case(DEFAULT_NAMESPACE)
        || schema.eq_ignore_ascii_case("pg_catalog")
    {
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

fn overload_suffix(routine: &ContractRoutine) -> String {
    let mut suffix = String::from("By");
    let mut any = false;
    for param in &routine.parameters {
        if !param.param_mode.is_input() {
            continue;
        }
        any = true;
        let ty = param.type_id.as_ref().map(|id| id.as_str()).unwrap_or(param.type_name.as_str());
        let short = ty.rsplit('.').next().unwrap_or(ty);
        suffix.push_str(&pascal_case(short));
        if param.is_array {
            suffix.push_str("List");
        }
    }
    if !any {
        suffix.push_str("Void");
    }
    suffix
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
        assert!(names.home_schema.is_none());
        assert_eq!(names.routine_ident("chat.create_message"), "ChatCreateMessage");
        assert_eq!(namespace_object_ident("chat"), "chat");
        assert_eq!(method_ident("create_message"), "createMessage");
    }

    #[test]
    fn home_schema_keeps_project_table_names_short() {
        let snapshot =
            compile_contract_sql("CREATE TABLE users (id BIGINT PRIMARY KEY);", "app").unwrap();
        let names = assign_names_with_home(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
            Some("app"),
        )
        .unwrap();
        assert_eq!(names.type_ident("app.users"), "Users");
        assert_eq!(value_ident("app", "users", names.uses_local_name("app")), "users");
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
