//! Module-level names the generated Python file binds, and the renames that
//! keep a generated class from taking one of them.
//!
//! Unlike Dart and TypeScript, Python has no import aliasing at the use site
//! and no separate type namespace: `class Enum(str, Enum)` rebinds the `Enum`
//! the file just imported, and the next generated enum fails to import with
//! "cannot extend enumeration". The fix is to rename the generated class, not
//! the import, so the rest of the target keeps using [`AssignedNames`] as it
//! does for every other language.

use std::collections::HashSet;

use kalamdb_sql::contracts::ContractSnapshot;

use super::{rows::SDK_ALIAS, RUNTIME_SYMBOLS};
use crate::workflow::schema::{naming::AssignedNames, output::unique_ident};

/// Names bound at module level by the file's own header and footer.
///
/// `annotations` is bound by `from __future__ import annotations`; the typing
/// and stdlib names are bound by the import block; `KALAM_TABLES` is the
/// registry the footer assigns. The runtime symbols are added from
/// [`RUNTIME_SYMBOLS`] so the two lists cannot drift.
const MODULE_BINDINGS: &[&str] = &[
    "Any",
    "Dict",
    "Enum",
    "KALAM_TABLES",
    "KalamClient",
    "KalamFunctions",
    "List",
    "Mapping",
    "Optional",
    "Union",
    "annotations",
    "dataclass",
    "dt",
    "TypedDict",
];

/// Every name the generated module binds outside its own declarations.
pub fn module_bindings() -> HashSet<String> {
    MODULE_BINDINGS
        .iter()
        .chain(RUNTIME_SYMBOLS.iter())
        .map(|name| (*name).to_string())
        .chain(std::iter::once(SDK_ALIAS.to_string()))
        .collect()
}

/// Generated names for the Python target, with shadowing idents renamed.
///
/// Only the type idents move. Procedure idents reach the file with a
/// `Request`/`Result`/`Input`/`Output` suffix, which no module binding has, and
/// table variables are already allocated against the runtime imports in
/// [`super::tables::table_bindings`].
pub fn python_names(snapshot: &ContractSnapshot, names: &AssignedNames) -> AssignedNames {
    let reserved = module_bindings();
    // Claim every assigned ident up front so a rename cannot land on a name
    // another type already owns.
    let mut used: HashSet<String> = names.types.values().cloned().collect();
    used.extend(reserved.iter().cloned());

    // Contract order (`snapshot.types` is keyed by qualified id), so the same
    // schema renames the same way on every run and `--check` stays stable.
    let mut renamed: Vec<(String, String)> = Vec::new();
    for id in snapshot.types.keys() {
        let ident = names.type_ident(id.as_str());
        if !reserved.contains(ident) || renamed.iter().any(|(from, _)| from == ident) {
            continue;
        }
        let replacement = unique_ident(ident.to_string(), &mut used);
        renamed.push((ident.to_string(), replacement));
    }

    if renamed.is_empty() {
        return names.clone();
    }

    let mut out = names.clone();
    for ident in out.types.values_mut() {
        if let Some((_, replacement)) = renamed.iter().find(|(from, _)| from == ident) {
            *ident = replacement.clone();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use kalamdb_sql::compile_contract_sql;

    use super::*;
    use crate::workflow::schema::naming::{assign_names, NamingOptions};

    fn names_for(sql: &str) -> (ContractSnapshot, AssignedNames) {
        let snapshot = compile_contract_sql(sql, "public").unwrap();
        let names = assign_names(
            &snapshot,
            NamingOptions {
                unqualified_names: false,
            },
        )
        .unwrap();
        (snapshot, names)
    }

    #[test]
    fn a_type_named_enum_does_not_take_the_import() {
        let (snapshot, names) = names_for("CREATE TYPE \"enum\" AS ENUM ('a');");
        let python = python_names(&snapshot, &names);
        assert_eq!(names.type_ident("public.enum"), "Enum");
        assert_eq!(python.type_ident("public.enum"), "Enum2");
    }

    #[test]
    fn a_rename_skips_a_name_another_type_already_owns() {
        let (snapshot, names) =
            names_for("CREATE TYPE \"enum\" AS ENUM ('a');\nCREATE TYPE enum2 AS (city TEXT);");
        let python = python_names(&snapshot, &names);
        assert_eq!(python.type_ident("public.enum2"), "Enum2");
        assert_eq!(python.type_ident("public.enum"), "Enum3");
    }

    #[test]
    fn a_type_named_after_a_runtime_import_is_renamed_too() {
        // The runtime import list is part of the module namespace, not just the
        // stdlib one: `class Table:` would replace the generic every table
        // declaration below it is annotated with.
        let (snapshot, names) = names_for("CREATE TYPE \"table\" AS (city TEXT);");
        let python = python_names(&snapshot, &names);
        assert_eq!(names.type_ident("public.table"), "Table");
        assert_eq!(python.type_ident("public.table"), "Table2");
    }

    #[test]
    fn untouched_schemas_keep_every_assigned_name() {
        let (snapshot, names) = names_for("CREATE TYPE mood AS ENUM ('calm');");
        assert_eq!(python_names(&snapshot, &names), names);
    }
}
