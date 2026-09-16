use std::{fs, path::Path, process::Command};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    bundle::{bundle_registry, empty_module_artifact, find_esbuild_bin, find_tsc_bin},
    packages::{lockfile_hash, validate_function_packages},
};
use crate::{
    error::{CLIError, Result},
    workflow::{
        schema::{
            compile_project_contract, generate_schema,
            procedure_bindings::discover_procedure_bindings, typescript::generate_registry_source,
        },
        WorkflowContext,
    },
};

pub async fn build_functions(ctx: &WorkflowContext) -> Result<()> {
    if !ctx.config.schema.languages.is_empty() {
        generate_schema(ctx, None)?;
    }
    validate_function_packages(&ctx.project_root)?;
    typecheck_functions(&ctx.project_root)?;
    write_module_artifact(ctx)?;
    write_build_manifest(ctx)?;
    ctx.output()
        .status(format!("built function module {}", ctx.config.functions.module));
    Ok(())
}

fn typecheck_functions(project_root: &Path) -> Result<()> {
    let src = project_root.join("functions/src");
    if !src.exists() {
        return Ok(());
    }
    reject_eval_in_tree(&src)?;
    let tsconfig = project_root.join("functions/tsconfig.json");
    if !tsconfig.exists() {
        return Ok(());
    }
    let Some(tsc) = find_tsc_bin(project_root) else {
        return Ok(());
    };
    let status = Command::new(&tsc)
        .arg("--noEmit")
        .arg("-p")
        .arg(&tsconfig)
        .current_dir(project_root.join("functions"))
        .status()
        .map_err(|error| CLIError::FileError(format!("failed to run tsc: {error}")))?;
    if !status.success() {
        return Err(CLIError::ConfigurationError("functions typecheck failed".into()));
    }
    Ok(())
}

fn reject_eval_in_tree(root: &Path) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("ts")
                && path.extension().and_then(|ext| ext.to_str()) != Some("js")
            {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|error| {
                CLIError::FileError(format!("failed to read '{}': {error}", path.display()))
            })?;
            if text.contains("eval(") || text.contains("new Function") {
                return Err(CLIError::ConfigurationError(format!(
                    "functions source '{}' must not use eval or Function",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn write_module_artifact(ctx: &WorkflowContext) -> Result<()> {
    let (snapshot, hash) = compile_project_contract(&ctx.project_root, &ctx.config)?;
    let bindings = discover_procedure_bindings(&ctx.project_root, &snapshot)?;
    let generated_dir = ctx.project_root.join("functions/src/generated");
    let registry_path = generated_dir.join("registry.ts");
    fs::create_dir_all(&generated_dir).map_err(|error| {
        CLIError::FileError(format!("failed to create '{}': {error}", generated_dir.display()))
    })?;
    fs::write(&registry_path, generate_registry_source(&hash, &bindings, &generated_dir)?)
        .map_err(|error| {
            CLIError::FileError(format!("failed to write '{}': {error}", registry_path.display()))
        })?;
    let implemented = bindings.iter().any(|binding| binding.implemented);
    let source = if implemented {
        let esbuild = find_esbuild_bin(&ctx.project_root).ok_or_else(|| {
            CLIError::ConfigurationError(
                "functions TypeScript with types requires esbuild; run npm install in the project \
                 root (Vite includes it) or add esbuild to functions/package.json"
                    .into(),
            )
        })?;
        bundle_registry(&esbuild, &registry_path)?
    } else {
        empty_module_artifact()?
    };
    let dir = ctx.project_root.join("functions/.kalam/build");
    fs::create_dir_all(&dir).map_err(|error| {
        CLIError::FileError(format!("failed to create '{}': {error}", dir.display()))
    })?;
    let path = dir.join("module.js");
    fs::write(&path, source).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", path.display()))
    })?;
    Ok(())
}

fn write_build_manifest(ctx: &WorkflowContext) -> Result<()> {
    let (snapshot, contract_hash) = compile_project_contract(&ctx.project_root, &ctx.config)?;
    let bindings = discover_procedure_bindings(&ctx.project_root, &snapshot)?;
    let implemented: std::collections::HashSet<&str> = bindings
        .iter()
        .filter(|binding| binding.implemented)
        .map(|binding| binding.routine_id.as_str())
        .collect();
    let mut procedures = serde_json::Map::new();
    for routine in snapshot.routines.values() {
        let kind = if implemented.contains(routine.routine_id.as_str()) {
            "module"
        } else if routine.body.is_some() {
            "inline"
        } else {
            "missing"
        };
        if kind == "missing" {
            ctx.output().warn(format!(
                "procedure {} is not implemented; CALL will fail until a handler exists under \
                 functions/src/",
                routine.routine_id
            ));
        }
        procedures.insert(routine.routine_id.to_string(), Value::String(kind.to_string()));
    }
    let registry = ctx.project_root.join("functions/src/generated/registry.ts");
    let artifact = ctx.project_root.join("functions/.kalam/build/module.js");
    let artifact_hash = if let Ok(bytes) = fs::read(&artifact) {
        hex::encode(Sha256::digest(bytes))
    } else if let Ok(bytes) = fs::read(&registry) {
        hex::encode(Sha256::digest(bytes))
    } else {
        contract_hash.clone()
    };
    let manifest = serde_json::json!({
        "manifestVersion": 1,
        "module": ctx.config.functions.module,
        "runtime": ctx.config.functions.runtime,
        "abiVersion": 2,
        "contractHash": contract_hash,
        "artifactHash": artifact_hash,
        "sourceHash": contract_hash,
        "lockfileHash": lockfile_hash(&ctx.project_root).unwrap_or_default(),
        "procedures": procedures,
    });
    let dir = ctx.project_root.join("functions/.kalam/build");
    fs::create_dir_all(&dir).map_err(|error| {
        CLIError::FileError(format!("failed to create '{}': {error}", dir.display()))
    })?;
    let path = dir.join("manifest.json");
    fs::write(&path, serde_json::to_string_pretty(&manifest).expect("manifest json")).map_err(
        |error| CLIError::FileError(format!("failed to write '{}': {error}", path.display())),
    )?;
    Ok(())
}

#[cfg(test)]
fn validate_registry_exports(
    project_root: &Path,
    snapshot: &kalamdb_sql::contracts::ContractSnapshot,
) -> Result<()> {
    discover_procedure_bindings(project_root, snapshot).map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn ignores_generated_runtime_declaration_files() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("functions/src/generated")).unwrap();
        fs::write(
            root.join("functions/src/generated/runtime.d.ts"),
            "export interface ProcedureContext {}\n",
        )
        .unwrap();
        fs::write(root.join("functions/src/generated/contracts.ts"), "export {}\n").unwrap();
        fs::write(root.join("functions/src/generated/registry.ts"), "export {}\n").unwrap();
        let snapshot = kalamdb_sql::contracts::ContractSnapshot::default();
        validate_registry_exports(root, &snapshot).unwrap();
    }

    #[test]
    fn ignores_helper_files_without_procedure_bindings() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("functions/src/api")).unwrap();
        fs::write(root.join("functions/src/api/orphan.ts"), "export const helper = 1;\n").unwrap();
        let mut snapshot = kalamdb_sql::contracts::ContractSnapshot::default();
        snapshot.routines.insert(
            "api.health".into(),
            kalamdb_sql::contracts::ContractRoutine {
                routine_id:  kalamdb_commons::models::RoutineId::new("api.health"),
                schema:      "api".into(),
                name:        "health".into(),
                parameters:  Vec::new(),
                return_type: None,
                language:    None,
                security:    kalamdb_commons::RoutineSecurityMode::Invoker,
                body:        None,
                grants:      Default::default(),
                comment:     None,
            },
        );
        validate_registry_exports(root, &snapshot).unwrap();
    }

    #[test]
    fn rejects_unknown_procedure_builders() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("functions/src/api")).unwrap();
        fs::write(
            root.join("functions/src/api/orphan.ts"),
            "import { procedure } from \"../generated/contracts\";\nexport const stale = \
             procedure.api.missing(async () => {});\n",
        )
        .unwrap();
        let mut snapshot = kalamdb_sql::contracts::ContractSnapshot::default();
        snapshot.routines.insert(
            "api.health".into(),
            kalamdb_sql::contracts::ContractRoutine {
                routine_id:  kalamdb_commons::models::RoutineId::new("api.health"),
                schema:      "api".into(),
                name:        "health".into(),
                parameters:  Vec::new(),
                return_type: None,
                language:    None,
                security:    kalamdb_commons::RoutineSecurityMode::Invoker,
                body:        None,
                grants:      Default::default(),
                comment:     None,
            },
        );
        let err = validate_registry_exports(root, &snapshot).unwrap_err();
        assert!(err.to_string().contains("unknown procedure builder"), "{err}");
    }

    #[test]
    fn manifest_classifies_module_inline_and_missing() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(
            root.join("schema.sql"),
            r#"
CREATE SCHEMA api;
CREATE PROCEDURE api.health() RETURNS TEXT;
CREATE PROCEDURE api.greet() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return "inline"; $$;
CREATE PROCEDURE api.plus_one(x INT) RETURNS INT;
"#,
        )
        .unwrap();
        let mut ctx = crate::workflow::test_support::test_workflow_context(root);
        ctx.config = crate::workflow::test_support::sql_project_config_with_typescript_target();
        crate::workflow::schema::gen::generate_languages(
            root,
            &ctx.config,
            &[crate::workflow::schema::LanguageTarget::TypeScript],
            None,
        )
        .unwrap();
        fs::write(
            root.join("functions/src/api/health.ts"),
            "import { procedure } from \"../generated/contracts\";\nexport const health = \
             procedure.api.health(async () => \"ok\");\n",
        )
        .unwrap();
        write_build_manifest(&ctx).unwrap();
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(root.join("functions/.kalam/build/manifest.json")).unwrap(),
        )
        .unwrap();
        let procedures = manifest.get("procedures").and_then(Value::as_object).unwrap();
        assert_eq!(procedures.get("api.health").and_then(Value::as_str), Some("module"));
        assert_eq!(procedures.get("api.greet").and_then(Value::as_str), Some("inline"));
        assert_eq!(procedures.get("api.plus_one").and_then(Value::as_str), Some("missing"));
    }
}
