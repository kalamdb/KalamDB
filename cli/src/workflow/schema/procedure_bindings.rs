//! AST discovery of named `procedure.<schema>.<method>` bindings.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use kalamdb_functions_host::{method_ident, namespace_object_ident};
use kalamdb_sql::contracts::ContractSnapshot;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, CallExpression, Declaration, Expression, ImportDeclaration,
    ImportDeclarationSpecifier, ModuleExportName, Statement, VariableDeclarationKind,
};
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::error::{CLIError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcedureBinding {
    pub routine_id:  String,
    pub schema:      String,
    pub name:        String,
    pub export_name: String,
    pub source_path: PathBuf,
    pub implemented: bool,
}

#[derive(Debug, Default)]
struct ProcedureImports {
    aliases:    HashSet<String>,
    namespaces: HashSet<String>,
}

pub fn discover_procedure_bindings(
    project_root: &Path,
    snapshot: &ContractSnapshot,
) -> Result<Vec<ProcedureBinding>> {
    let identities = identity_map(snapshot);
    let mut bindings = Vec::new();
    let mut seen: HashMap<String, ProcedureBinding> = HashMap::new();
    for path in procedure_source_files(project_root)? {
        let file_bindings = discover_file_bindings(&path, project_root, &identities)?;
        for binding in file_bindings {
            if let Some(previous) = seen.get(&binding.routine_id) {
                return Err(CLIError::ConfigurationError(format!(
                    "duplicate procedure binding for {} (export '{}' in '{}' and export '{}' in \
                     '{}')",
                    binding.routine_id,
                    previous.export_name,
                    display_rel(project_root, &previous.source_path),
                    binding.export_name,
                    display_rel(project_root, &binding.source_path),
                )));
            }
            seen.insert(binding.routine_id.clone(), binding.clone());
            bindings.push(binding);
        }
    }
    bindings.sort_by(|left, right| left.routine_id.cmp(&right.routine_id));
    Ok(bindings)
}

pub fn implemented_bindings(bindings: &[ProcedureBinding]) -> Vec<&ProcedureBinding> {
    bindings.iter().filter(|binding| binding.implemented).collect()
}

fn identity_map(
    snapshot: &ContractSnapshot,
) -> BTreeMap<(String, String), (String, String, String)> {
    snapshot
        .routines
        .values()
        .map(|routine| {
            (
                (namespace_object_ident(&routine.schema), method_ident(&routine.name)),
                (routine.routine_id.to_string(), routine.schema.clone(), routine.name.clone()),
            )
        })
        .collect()
}

fn procedure_source_files(project_root: &Path) -> Result<Vec<PathBuf>> {
    let src = project_root.join("functions").join("src");
    if !src.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|error| {
            CLIError::FileError(format!("failed to read '{}': {error}", dir.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                CLIError::FileError(format!("failed to read '{}': {error}", dir.display()))
            })?;
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some("generated") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if is_generated_rel(&src, &path) {
                continue;
            }
            if !is_procedure_source(&path) {
                continue;
            }
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn is_generated_rel(src: &Path, path: &Path) -> bool {
    path.strip_prefix(src)
        .map(|rel| rel.components().any(|component| component.as_os_str() == "generated"))
        .unwrap_or(false)
}

fn is_procedure_source(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.ends_with(".d.ts") {
        return false;
    }
    matches!(path.extension().and_then(|ext| ext.to_str()), Some("ts" | "js" | "mts" | "mjs"))
}

fn discover_file_bindings(
    path: &Path,
    project_root: &Path,
    identities: &BTreeMap<(String, String), (String, String, String)>,
) -> Result<Vec<ProcedureBinding>> {
    let source = fs::read_to_string(path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", path.display()))
    })?;
    let allocator = Allocator::new();
    let source_type =
        SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts().with_module(true));
    let parsed = Parser::new(&allocator, &source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        let first = parsed
            .diagnostics
            .errors()
            .next()
            .map(ToString::to_string)
            .unwrap_or_else(|| "parse error".to_string());
        return Err(CLIError::ConfigurationError(format!(
            "failed to parse procedure source '{}': {first}",
            display_rel(project_root, path),
        )));
    }

    let mut imports = ProcedureImports::default();
    let mut bindings = Vec::new();
    for statement in &parsed.program.body {
        match statement {
            Statement::ImportDeclaration(decl) => collect_imports(decl, &mut imports),
            Statement::ExportNamedDeclaration(decl) => {
                let Some(Declaration::VariableDeclaration(var)) = &decl.declaration else {
                    continue;
                };
                if var.kind != VariableDeclarationKind::Const {
                    continue;
                }
                for declarator in &var.declarations {
                    let Some(export_name) = binding_ident_name(&declarator.id) else {
                        continue;
                    };
                    let Some(init) = &declarator.init else {
                        continue;
                    };
                    match classify_procedure_init(init, &imports) {
                        InitKind::Ignore => {},
                        InitKind::Malformed => {
                            return Err(CLIError::ConfigurationError(format!(
                                "malformed procedure binding '{}' in '{}'; export a generated \
                                 `procedure.<schema>.<method>(handler)` or `.unimplemented()` call",
                                export_name,
                                display_rel(project_root, path),
                            )));
                        },
                        InitKind::Binding {
                            namespace,
                            method,
                            implemented,
                        } => {
                            let Some((routine_id, schema, name)) =
                                identities.get(&(namespace.clone(), method.clone()))
                            else {
                                return Err(CLIError::ConfigurationError(format!(
                                    "unknown procedure builder procedure.{namespace}.{method} \
                                     (export '{export_name}' in '{}')",
                                    display_rel(project_root, path),
                                )));
                            };
                            bindings.push(ProcedureBinding {
                                routine_id: routine_id.clone(),
                                schema: schema.clone(),
                                name: name.clone(),
                                export_name: export_name.to_string(),
                                source_path: path.to_path_buf(),
                                implemented,
                            });
                        },
                    }
                }
            },
            _ => {},
        }
    }
    Ok(bindings)
}

fn collect_imports(decl: &ImportDeclaration<'_>, imports: &mut ProcedureImports) {
    let Some(specifiers) = &decl.specifiers else {
        return;
    };
    for specifier in specifiers {
        match specifier {
            ImportDeclarationSpecifier::ImportSpecifier(spec) => {
                if imported_name(&spec.imported) == "procedure" {
                    imports.aliases.insert(spec.local.name.as_str().to_string());
                }
            },
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(spec) => {
                imports.namespaces.insert(spec.local.name.as_str().to_string());
            },
            ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {},
        }
    }
}

fn imported_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.as_str().to_string(),
        ModuleExportName::IdentifierReference(id) => id.name.as_str().to_string(),
        ModuleExportName::StringLiteral(literal) => literal.value.as_str().to_string(),
    }
}

fn binding_ident_name<'a>(pattern: &'a BindingPattern<'a>) -> Option<&'a str> {
    match pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}

enum InitKind {
    Ignore,
    Malformed,
    Binding {
        namespace:   String,
        method:      String,
        implemented: bool,
    },
}

fn classify_procedure_init(expr: &Expression<'_>, imports: &ProcedureImports) -> InitKind {
    let expr = unwrap_expr(expr);
    if let Expression::CallExpression(call) = expr {
        return classify_call(call, imports);
    }
    if member_chain(expr).is_some_and(|chain| chain_starts_with_procedure(&chain, imports)) {
        return InitKind::Malformed;
    }
    InitKind::Ignore
}

fn classify_call(call: &CallExpression<'_>, imports: &ProcedureImports) -> InitKind {
    let Some(chain) = member_chain(&call.callee) else {
        return InitKind::Ignore;
    };
    if !chain_starts_with_procedure(&chain, imports) {
        return InitKind::Ignore;
    }
    match identity_from_chain(&chain, imports) {
        Some((namespace, method, implemented)) => InitKind::Binding {
            namespace,
            method,
            implemented,
        },
        None => InitKind::Malformed,
    }
}

fn chain_starts_with_procedure(chain: &[&str], imports: &ProcedureImports) -> bool {
    chain.first().is_some_and(|first| {
        imports.aliases.contains(*first) || imports.namespaces.contains(*first)
    })
}

fn identity_from_chain(
    chain: &[&str],
    imports: &ProcedureImports,
) -> Option<(String, String, bool)> {
    let stripped = if chain.first().is_some_and(|first| imports.namespaces.contains(*first)) {
        if chain.get(1) != Some(&"procedure") {
            return None;
        }
        &chain[2..]
    } else if chain.first().is_some_and(|first| imports.aliases.contains(*first)) {
        &chain[1..]
    } else {
        return None;
    };
    match stripped {
        [namespace, method] => Some((namespace.to_string(), method.to_string(), true)),
        [namespace, method, "unimplemented"] => {
            Some((namespace.to_string(), method.to_string(), false))
        },
        _ => None,
    }
}

fn member_chain<'a>(expr: &'a Expression<'a>) -> Option<Vec<&'a str>> {
    let expr = unwrap_expr(expr);
    match expr {
        Expression::Identifier(id) => Some(vec![id.name.as_str()]),
        Expression::StaticMemberExpression(member) => {
            let mut chain = member_chain(&member.object)?;
            chain.push(member.property.name.as_str());
            Some(chain)
        },
        _ => None,
    }
}

fn unwrap_expr<'a>(expr: &'a Expression<'a>) -> &'a Expression<'a> {
    match expr {
        Expression::ParenthesizedExpression(inner) => unwrap_expr(&inner.expression),
        Expression::TSAsExpression(inner) => unwrap_expr(&inner.expression),
        Expression::TSSatisfiesExpression(inner) => unwrap_expr(&inner.expression),
        Expression::TSNonNullExpression(inner) => unwrap_expr(&inner.expression),
        other => other,
    }
}

fn display_rel(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::RoutineId;
    use kalamdb_sql::contracts::ContractRoutine;
    use tempfile::TempDir;

    use super::*;

    fn snapshot_with(routines: &[(&str, &str)]) -> ContractSnapshot {
        let mut snapshot = ContractSnapshot::default();
        for (schema, name) in routines {
            let routine_id = format!("{schema}.{name}");
            snapshot.routines.insert(
                routine_id.clone(),
                ContractRoutine {
                    routine_id:  RoutineId::new(&routine_id),
                    schema:      (*schema).into(),
                    name:        (*name).into(),
                    parameters:  Vec::new(),
                    return_type: None,
                    language:    None,
                    security:    kalamdb_commons::RoutineSecurityMode::Invoker,
                    body:        None,
                    grants:      Default::default(),
                    comment:     None,
                },
            );
        }
        snapshot
    }

    fn write_src(root: &Path, rel: &str, source: &str) {
        let path = root.join("functions/src").join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    #[test]
    fn discovers_two_named_bindings_in_one_file() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_src(
            root,
            "chat/chat.ts",
            r#"
import { procedure } from "../generated/contracts";
export const send = procedure.chat.sendMessage(async (ctx, input) => input);
export const join = procedure.chat.joinRoom(async (ctx, input) => input);
export function helper() { return 1; }
"#,
        );
        let snapshot = snapshot_with(&[("chat", "send_message"), ("chat", "join_room")]);
        let bindings = discover_procedure_bindings(root, &snapshot).unwrap();
        assert_eq!(bindings.len(), 2);
        assert!(bindings.iter().all(|binding| binding.implemented));
        assert!(bindings
            .iter()
            .any(|binding| binding.routine_id == "chat.send_message"
                && binding.export_name == "send"));
        assert!(
            bindings
                .iter()
                .any(|binding| binding.routine_id == "chat.join_room"
                    && binding.export_name == "join")
        );
    }

    #[test]
    fn accepts_import_alias_and_omits_unimplemented() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_src(
            root,
            "api/health.ts",
            r#"
import { procedure as p } from "../generated/procedure.js";
export const health = p.api.health.unimplemented();
"#,
        );
        let snapshot = snapshot_with(&[("api", "health")]);
        let bindings = discover_procedure_bindings(root, &snapshot).unwrap();
        assert_eq!(bindings.len(), 1);
        assert!(!bindings[0].implemented);
        assert_eq!(bindings[0].export_name, "health");
    }

    #[test]
    fn accepts_namespace_import() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_src(
            root,
            "api/health.ts",
            r#"
import * as contracts from "../generated/contracts";
export const health = contracts.procedure.api.health(async () => "ok");
"#,
        );
        let snapshot = snapshot_with(&[("api", "health")]);
        let bindings = discover_procedure_bindings(root, &snapshot).unwrap();
        assert_eq!(bindings[0].routine_id, "api.health");
        assert!(bindings[0].implemented);
    }

    #[test]
    fn rejects_unknown_and_duplicate_builders() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_src(
            root,
            "api/one.ts",
            r#"
import { procedure } from "../generated/contracts";
export const stale = procedure.api.missing(async () => {});
"#,
        );
        let snapshot = snapshot_with(&[("api", "health")]);
        let err = discover_procedure_bindings(root, &snapshot).unwrap_err();
        assert!(err.to_string().contains("unknown procedure builder"), "{err}");

        fs::write(
            root.join("functions/src/api/one.ts"),
            r#"
import { procedure } from "../generated/contracts";
export const health = procedure.api.health(async () => "ok");
"#,
        )
        .unwrap();
        write_src(
            root,
            "api/two.ts",
            r#"
import { procedure } from "../generated/contracts";
export const also = procedure.api.health(async () => "ok");
"#,
        );
        let err = discover_procedure_bindings(root, &snapshot).unwrap_err();
        assert!(err.to_string().contains("duplicate procedure binding"), "{err}");
    }

    #[test]
    fn ignores_helpers_and_rejects_malformed_chains() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_src(
            root,
            "api/health.ts",
            r#"
import { procedure } from "../generated/contracts";
export const answer = 42;
export const broken = procedure.api();
"#,
        );
        let snapshot = snapshot_with(&[("api", "health")]);
        let err = discover_procedure_bindings(root, &snapshot).unwrap_err();
        assert!(err.to_string().contains("malformed procedure binding"), "{err}");
    }
}
