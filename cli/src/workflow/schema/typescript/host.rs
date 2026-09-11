//! TypeScript host artifacts: runtime d.ts/js, procedure builders, registry, scaffolds.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use kalamdb_sql::contracts::{ContractRoutine, ContractSnapshot};
use serde::Serialize;

use super::text::ts_relative_module;
use crate::{
    error::{CLIError, Result},
    workflow::{
        project::templates::{escape_double_quoted_string, render_schema_gen_file},
        schema::{
            naming::{method_ident, namespace_object_ident, value_ident, AssignedNames},
            output::{remove_if_exists, write_text, GeneratedHeader, GENERATED_HEADER},
            procedure_bindings::{discover_procedure_bindings, ProcedureBinding},
            procedures::ProcedureCatalog,
        },
    },
};

const FUNCTIONS_DIR: &str = "functions";
const RUNTIME_JS_TEMPLATE: &str = "functions/src/generated/runtime.js";
const RUNTIME_DTS_TEMPLATE: &str = "functions/src/generated/runtime.d.ts";
const PROCEDURE_DTS_TEMPLATE: &str = "functions/src/generated/procedure.d.ts";
const PROCEDURE_JS_TEMPLATE: &str = "functions/src/generated/procedure.js";
const CONTRACTS_TEMPLATE: &str = "functions/src/generated/contracts.ts";
const REGISTRY_TEMPLATE: &str = "functions/src/generated/registry.ts";
const INLINE_TEMPLATE: &str = "functions/src/generated/inline.ts";
const UNIMPLEMENTED_TEMPLATE: &str = "functions/src/procedure.unimplemented.ts";
const IMPLEMENTED_TEMPLATE: &str = "functions/src/procedure.implemented.ts";

#[derive(Serialize)]
struct HostNamespacesContext {
    header:           &'static str,
    has_host:         bool,
    has_type_imports: bool,
    has_namespaces:   bool,
    type_imports:     String,
    schema_import:    String,
    namespaces:       Vec<HostNamespace>,
}

#[derive(Serialize)]
struct HostNamespace {
    object_ident: String,
    procedures:   Vec<HostProcedure>,
}

#[derive(Serialize)]
struct HostProcedure {
    method_ident: String,
    type_ident:   String,
    has_params:   bool,
}

#[derive(Serialize)]
struct ContractsContext<'a> {
    #[serde(flatten)]
    file:          GeneratedHeader,
    schema_import: &'a str,
}

#[derive(Serialize)]
struct RegistryContext {
    #[serde(flatten)]
    file:       GeneratedHeader,
    imports:    Vec<RegistryImport>,
    procedures: Vec<RegistryProcedure>,
}

#[derive(Serialize)]
struct RegistryImport {
    names: String,
    from:  String,
}

#[derive(Serialize)]
struct RegistryProcedure {
    routine_id: String,
    ident:      String,
}

#[derive(Serialize)]
struct ProcedureScaffoldContext<'a> {
    ns:       String,
    method:   String,
    has_body: bool,
    body:     &'a str,
}

#[derive(Serialize)]
struct InlineContext<'a> {
    header: &'static str,
    ident:  String,
    body:   &'a str,
}

pub fn write_procedure_artifacts(
    project_root: &Path,
    schema_path: &Path,
    snapshot: &ContractSnapshot,
    hash: &str,
    procedures: &ProcedureCatalog<'_>,
) -> Result<()> {
    let generated_dir = project_root.join(FUNCTIONS_DIR).join("src").join("generated");
    let schema_import = ts_relative_module(&generated_dir, schema_path);
    write_text(
        &generated_dir.join("runtime.d.ts"),
        &render_runtime_dts(procedures, &schema_import)?,
    )?;
    // Identity helper must be `.js`. A sibling `runtime.ts` shadows `runtime.d.ts`
    // and types `wrapProcedure` as `(handler: any) => any`.
    write_text(&generated_dir.join("runtime.js"), &generate_runtime_js()?)?;
    remove_if_exists(&generated_dir.join("runtime.ts"))?;
    remove_stale_dot_kalam_generated(project_root)?;
    if procedures.is_empty() {
        return Ok(());
    }
    write_text(
        &generated_dir.join("procedure.d.ts"),
        &render_procedure_dts(procedures, &schema_import)?,
    )?;
    write_text(&generated_dir.join("procedure.js"), &render_procedure_js(procedures)?)?;
    remove_if_exists(&generated_dir.join("procedure.ts"))?;
    write_text(
        &generated_dir.join("contracts.ts"),
        &generate_contracts_source(hash, &schema_import)?,
    )?;

    let bindings = discover_procedure_bindings(project_root, snapshot)?;
    let bound: HashSet<&str> = bindings.iter().map(|binding| binding.routine_id.as_str()).collect();
    for spec in procedures.procedures() {
        if is_project_backed_routine(spec.routine)
            && !bound.contains(spec.routine.routine_id.as_str())
        {
            scaffold_procedure(project_root, spec.routine)?;
        }
        write_inline_shim(&generated_dir, spec.routine)?;
    }
    let bindings = discover_procedure_bindings(project_root, snapshot)?;
    write_text(
        &generated_dir.join("registry.ts"),
        &generate_registry_source(hash, &bindings, &generated_dir)?,
    )?;
    Ok(())
}

pub fn generate_contracts_source(hash: &str, schema_import: &str) -> Result<String> {
    render_schema_gen_file(
        "typescript",
        CONTRACTS_TEMPLATE,
        &ContractsContext {
            file:          GeneratedHeader::from_hash(hash),
            schema_import: &escape_double_quoted_string(schema_import),
        },
    )
}

pub fn generate_runtime_dts(
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    schema_import: &str,
) -> Result<String> {
    let procedures = ProcedureCatalog::from_snapshot(snapshot, names);
    render_runtime_dts(&procedures, schema_import)
}

fn render_runtime_dts(procedures: &ProcedureCatalog<'_>, schema_import: &str) -> Result<String> {
    render_schema_gen_file(
        "typescript",
        RUNTIME_DTS_TEMPLATE,
        &host_context(procedures, schema_import, false),
    )
}

pub fn generate_runtime_js() -> Result<String> {
    render_schema_gen_file(
        "typescript",
        RUNTIME_JS_TEMPLATE,
        &serde_json::json!({ "header": GENERATED_HEADER }),
    )
}

fn render_procedure_dts(procedures: &ProcedureCatalog<'_>, schema_import: &str) -> Result<String> {
    render_schema_gen_file(
        "typescript",
        PROCEDURE_DTS_TEMPLATE,
        &host_context(procedures, schema_import, true),
    )
}

fn render_procedure_js(procedures: &ProcedureCatalog<'_>) -> Result<String> {
    render_schema_gen_file(
        "typescript",
        PROCEDURE_JS_TEMPLATE,
        &host_context(procedures, "", false),
    )
}

fn host_context(
    procedures: &ProcedureCatalog<'_>,
    schema_import: &str,
    include_empty_request: bool,
) -> HostNamespacesContext {
    let namespaces = host_namespaces(procedures);
    let type_imports = procedures.schema_type_imports(include_empty_request);
    HostNamespacesContext {
        header: GENERATED_HEADER,
        has_host: !namespaces.is_empty(),
        has_type_imports: !type_imports.is_empty(),
        has_namespaces: !namespaces.is_empty(),
        type_imports,
        schema_import: escape_double_quoted_string(schema_import),
        namespaces,
    }
}

fn host_namespaces(procedures: &ProcedureCatalog<'_>) -> Vec<HostNamespace> {
    procedures
        .namespaces()
        .iter()
        .map(|namespace| HostNamespace {
            object_ident: namespace.object_ident.clone(),
            procedures:   namespace
                .procedures
                .iter()
                .map(|spec| HostProcedure {
                    method_ident: spec.method_ident.clone(),
                    type_ident:   spec.type_ident.to_string(),
                    has_params:   spec.has_params(),
                })
                .collect(),
        })
        .collect()
}

pub fn generate_registry_source(
    hash: &str,
    bindings: &[ProcedureBinding],
    generated_dir: &Path,
) -> Result<String> {
    let implemented: Vec<&ProcedureBinding> =
        bindings.iter().filter(|binding| binding.implemented).collect();
    let mut by_file: BTreeMap<&Path, Vec<&ProcedureBinding>> = BTreeMap::new();
    for binding in &implemented {
        by_file.entry(binding.source_path.as_path()).or_default().push(*binding);
    }
    let imports = by_file
        .iter()
        .map(|(path, file_bindings)| RegistryImport {
            names: file_bindings
                .iter()
                .map(|binding| {
                    format!(
                        "{} as {}",
                        binding.export_name,
                        value_ident(&binding.schema, &binding.name, false)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
            from:  escape_double_quoted_string(&ts_relative_module(generated_dir, path)),
        })
        .collect();
    let procedures = implemented
        .iter()
        .map(|binding| RegistryProcedure {
            routine_id: escape_double_quoted_string(&binding.routine_id),
            ident:      value_ident(&binding.schema, &binding.name, false),
        })
        .collect();
    render_schema_gen_file(
        "typescript",
        REGISTRY_TEMPLATE,
        &RegistryContext {
            file: GeneratedHeader::from_hash(hash),
            imports,
            procedures,
        },
    )
}

fn write_inline_shim(generated_dir: &Path, routine: &ContractRoutine) -> Result<()> {
    let Some(body) = routine.body.as_deref() else {
        return Ok(());
    };
    let ident = format!("{}_{}", routine.schema, routine.name);
    let path = generated_dir.join("inline").join(format!("{ident}.ts"));
    let source = render_schema_gen_file(
        "typescript",
        INLINE_TEMPLATE,
        &InlineContext {
            header: GENERATED_HEADER,
            ident,
            body,
        },
    )?;
    write_text(&path, &source)
}

fn scaffold_procedure(project_root: &Path, routine: &ContractRoutine) -> Result<()> {
    let path = procedure_impl_path(project_root, &routine.schema, &routine.name);
    if path.exists() {
        return Ok(());
    }
    write_text(&path, &unimplemented_procedure_source(&routine.schema, &routine.name)?)
}

fn unimplemented_procedure_source(schema: &str, name: &str) -> Result<String> {
    render_schema_gen_file(
        "typescript",
        UNIMPLEMENTED_TEMPLATE,
        &ProcedureScaffoldContext {
            ns:       namespace_object_ident(schema),
            method:   method_ident(name),
            has_body: false,
            body:     "",
        },
    )
}

pub fn implemented_procedure_source(
    schema: &str,
    name: &str,
    body: Option<&str>,
) -> Result<String> {
    let has_body = body.is_some_and(|body| !body.trim().is_empty());
    render_schema_gen_file(
        "typescript",
        IMPLEMENTED_TEMPLATE,
        &ProcedureScaffoldContext {
            ns: namespace_object_ident(schema),
            method: method_ident(name),
            has_body,
            body: body.unwrap_or(""),
        },
    )
}

pub fn procedure_impl_path(project_root: &Path, schema: &str, name: &str) -> PathBuf {
    project_root
        .join(FUNCTIONS_DIR)
        .join("src")
        .join(schema)
        .join(format!("{name}.ts"))
}

fn is_project_backed_routine(routine: &ContractRoutine) -> bool {
    routine.body.is_none() && is_typescript_routine(routine)
}

fn is_typescript_routine(routine: &ContractRoutine) -> bool {
    match routine.language.as_deref() {
        None => true,
        Some(language) => matches!(
            language.to_ascii_uppercase().as_str(),
            "TS" | "TYPESCRIPT" | "JS" | "JAVASCRIPT" | "PLV8"
        ),
    }
}

fn remove_stale_dot_kalam_generated(project_root: &Path) -> Result<()> {
    let stale = project_root.join(FUNCTIONS_DIR).join(".kalam").join("generated");
    if !stale.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&stale).map_err(|error| {
        CLIError::FileError(format!("failed to remove '{}': {error}", stale.display()))
    })
}
