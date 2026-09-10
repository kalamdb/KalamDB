//! `kalam functions` build/status/rollback/logs.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use kalam_client::AuthProvider;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    error::{CLIError, Result},
    workflow::{
        auth::{login_with_credentials, resolve_workflow_auth_provider},
        generate_schema,
        schema::{
            compile_project_contract,
            procedure_bindings::discover_procedure_bindings,
            typescript::{
                generate_registry_source, implemented_procedure_source, procedure_impl_path,
            },
        },
        sql::{build_workflow_client, execute_and_print},
        WorkflowContext,
    },
};

const NODE_BUILTINS: &[&str] = &[
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

pub async fn build_functions(ctx: &WorkflowContext) -> Result<()> {
    if !ctx.config.schema.languages.is_empty() {
        generate_schema(ctx, None)?;
    }
    validate_function_packages(&ctx.project_root)?;
    typecheck_functions(&ctx.project_root)?;
    write_module_artifact(ctx)?;
    write_build_manifest(ctx)?;
    Ok(())
}

fn typecheck_functions(project_root: &Path) -> Result<()> {
    let src = project_root.join("functions/src");
    if !src.exists() {
        return Ok(());
    }
    reject_eval_in_tree(&src)?;
    let tsconfig = project_root.join("functions/tsconfig.json");
    let tsc = project_root.join("functions/node_modules/.bin/tsc");
    if tsconfig.exists() && tsc.exists() {
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
    fs::write(&registry_path, generate_registry_source(&hash, &bindings, &generated_dir)).map_err(
        |error| {
            CLIError::FileError(format!("failed to write '{}': {error}", registry_path.display()))
        },
    )?;
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
        empty_module_artifact()
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

fn find_esbuild_bin(project_root: &Path) -> Option<PathBuf> {
    for rel in [
        "node_modules/esbuild/bin/esbuild",
        "functions/node_modules/esbuild/bin/esbuild",
    ] {
        let path = project_root.join(rel);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn esbuild_resolve_root(source: &Path) -> Option<&Path> {
    let mut package_json_dir = None;
    for ancestor in source.ancestors() {
        if ancestor.join("kalam.toml").is_file() {
            return Some(ancestor);
        }
        if package_json_dir.is_none() && ancestor.join("package.json").is_file() {
            package_json_dir = Some(ancestor);
        }
    }
    package_json_dir
}

fn esbuild_node_paths(source: &Path) -> Option<String> {
    let root = esbuild_resolve_root(source)?;
    let mut dirs = Vec::new();
    if root.file_name().and_then(|name| name.to_str()) == Some("functions") {
        if let Some(parent_modules) = root.parent().map(|parent| parent.join("node_modules")) {
            if parent_modules.is_dir() {
                dirs.push(parent_modules);
            }
        }
    }
    for rel in ["node_modules", "functions/node_modules"] {
        let path = root.join(rel);
        if path.is_dir() {
            dirs.push(path);
        }
    }
    if dirs.is_empty() {
        return None;
    }
    let abs: Vec<PathBuf> = dirs.into_iter().filter_map(|path| path.canonicalize().ok()).collect();
    if abs.is_empty() {
        return None;
    }
    std::env::join_paths(&abs).ok()?.into_string().ok()
}

fn esbuild_temp_path(kind: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "kalam-fn-{kind}-{}-{}.js",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    path
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

fn path_as_esbuild_import(path: &Path) -> Result<String> {
    let canonical = path.canonicalize().map_err(|error| {
        CLIError::FileError(format!("failed to resolve '{}': {error}", path.display()))
    })?;
    let utf8 = canonical.to_str().ok_or_else(|| {
        CLIError::FileError(format!("procedure path is not utf-8: {}", canonical.display()))
    })?;
    Ok(js_string(&utf8.replace('\\', "/")))
}

fn esbuild_run_bundle(
    esbuild_bin: &Path,
    source: &Path,
    format: &str,
    minify: bool,
    node_path_from: &Path,
) -> Result<String> {
    let sourcefile = source.to_str().ok_or_else(|| {
        CLIError::FileError(format!("procedure path is not utf-8: {}", source.display()))
    })?;
    // esbuild `--outfile=-` writes a file named `-`, not stdout.
    let outfile = esbuild_temp_path("out");
    let outfile_arg = outfile
        .to_str()
        .ok_or_else(|| CLIError::FileError("esbuild tempfile path is not utf-8".into()))?;
    // `file:`-linked packages (e.g. `@kalamdb/orm`) resolve from their real path, so
    // peer deps like `drizzle-orm` live in the app `node_modules`, not the SDK tree.
    // esbuild's CLI reads extra package search directories from NODE_PATH.
    let mut command = Command::new(esbuild_bin);
    command.args([
        sourcefile,
        "--bundle",
        &format!("--format={format}"),
        "--platform=neutral",
        "--target=es2022",
        "--log-level=error",
        "--legal-comments=none",
        &format!("--outfile={outfile_arg}"),
    ]);
    if minify {
        command.arg("--minify");
    }
    if let Some(node_paths) = esbuild_node_paths(node_path_from) {
        command.env("NODE_PATH", node_paths);
    }
    let output = command
        .output()
        .map_err(|error| CLIError::FileError(format!("failed to start esbuild: {error}")))?;
    if !output.status.success() {
        let _ = fs::remove_file(&outfile);
        return Err(CLIError::ConfigurationError(format!(
            "esbuild failed for {sourcefile}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let bundled = fs::read_to_string(&outfile)
        .map_err(|error| CLIError::FileError(format!("failed to read esbuild output: {error}")))?;
    let _ = fs::remove_file(&outfile);
    Ok(bundled)
}

fn kalam_invoke_source() -> &'static str {
    "function kalamInvoke(name, args) {\n  const ctx = globalThis.__kalamCtx;\n  const input = \
     args.length === 1 ? args[0] : Array.from(args);\n  const fn = procedures[name];\n  if (typeof \
     fn !== \"function\") {\n    throw new Error(\"missing export \" + name);\n  }\n  return \
     fn(ctx, input);\n}\nglobalThis.kalamInvoke = kalamInvoke;\n"
}

fn empty_module_artifact() -> String {
    format!("const procedures = {{}};\n{}", kalam_invoke_source())
}

fn bundle_registry(esbuild_bin: &Path, registry: &Path) -> Result<String> {
    let mut entry = String::from("import { procedures } from ");
    entry.push_str(&path_as_esbuild_import(registry)?);
    entry.push_str(";\n");
    entry.push_str(kalam_invoke_source());
    let entry_path = esbuild_temp_path("entry");
    let written = fs::write(&entry_path, &entry)
        .map_err(|error| CLIError::FileError(format!("failed to write esbuild entry: {error}")));
    if let Err(error) = written {
        let _ = fs::remove_file(&entry_path);
        return Err(error);
    }
    let bundled = esbuild_run_bundle(esbuild_bin, &entry_path, "iife", true, registry);
    let _ = fs::remove_file(&entry_path);
    bundled
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

pub fn validate_function_packages(project_root: &Path) -> Result<()> {
    let functions_dir = project_root.join("functions");
    reject_native_addons(&functions_dir)?;
    let package_json = functions_dir.join("package.json");
    if !package_json.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&package_json).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", package_json.display()))
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        CLIError::ConfigurationError(format!("invalid functions/package.json: {error}"))
    })?;
    for name in dependency_names(&value) {
        if is_forbidden_package(&name) {
            return Err(CLIError::ConfigurationError(format!(
                "functions package '{name}' is not allowed (Node builtin or native N-API addon)"
            )));
        }
    }
    Ok(())
}

pub fn lockfile_hash(project_root: &Path) -> Option<String> {
    for name in [
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
    ] {
        let path = project_root.join("functions").join(name);
        if let Ok(bytes) = fs::read(&path) {
            return Some(hex::encode(Sha256::digest(bytes)));
        }
    }
    None
}

fn dependency_names(value: &Value) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for key in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        if let Some(Value::Object(map)) = value.get(key) {
            names.extend(map.keys().cloned());
        }
    }
    names
}

fn is_forbidden_package(name: &str) -> bool {
    let name = name.trim_start_matches("node:");
    NODE_BUILTINS.iter().any(|builtin| *builtin == name)
        || name.ends_with(".node")
        || name == "bindings"
        || name == "node-gyp"
        || name == "node-addon-api"
        || name == "nan"
}

fn reject_native_addons(functions_dir: &Path) -> Result<()> {
    if !functions_dir.exists() {
        return Ok(());
    }
    let mut stack = vec![functions_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) == Some("node_modules") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) == Some("node") {
                return Err(CLIError::ConfigurationError(format!(
                    "native addon '{}' is not allowed in functions/",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

pub async fn show_function_status(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    let namespace = Some(env.namespace.as_str());
    output.status("active function module");
    execute_and_print(
        ctx,
        &client,
        "SELECT module_id, current_revision_id, runtime, abi_version, contract_hash FROM \
         system.modules ORDER BY module_id",
        namespace,
        "functions status",
    )
    .await?;
    output.status("procedures");
    execute_and_print(
        ctx,
        &client,
        "SELECT procedure_id, implementation, module_id, revision_id, security, signature, \
         return_type, grants FROM system.procedures ORDER BY procedure_id",
        namespace,
        "functions status",
    )
    .await?;
    output.status("function runtime");
    execute_and_print(
        ctx,
        &client,
        "SELECT metric_name, metric_value FROM system.stats WHERE metric_name LIKE \
         'function_memory%' OR metric_name LIKE 'function_instances%' ORDER BY metric_name",
        namespace,
        "functions status",
    )
    .await
}

pub async fn show_function_revisions(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    output.status("module revisions");
    execute_and_print(
        ctx,
        &client,
        "SELECT module_id, revision_id, is_current, contract_hash, artifact_bytes, created_at \
         FROM system.module_revisions ORDER BY created_at DESC",
        Some(env.namespace.as_str()),
        "functions revisions",
    )
    .await
}

pub async fn activate_function_module(ctx: &WorkflowContext) -> Result<()> {
    let artifact_path = ctx.project_root.join("functions/.kalam/build/module.js");
    if !artifact_path.is_file() {
        return Ok(());
    }
    let manifest_path = ctx.project_root.join("functions/.kalam/build/manifest.json");
    let manifest: Value = if manifest_path.is_file() {
        serde_json::from_str(&fs::read_to_string(&manifest_path).map_err(|error| {
            CLIError::FileError(format!("failed to read '{}': {error}", manifest_path.display()))
        })?)
        .map_err(|error| {
            CLIError::ConfigurationError(format!("invalid functions manifest: {error}"))
        })?
    } else {
        serde_json::json!({})
    };
    let artifact = fs::read_to_string(&artifact_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", artifact_path.display()))
    })?;
    let module = manifest
        .get("module")
        .and_then(Value::as_str)
        .unwrap_or(ctx.config.functions.module.as_str());
    let contract_hash = manifest
        .get("contractHash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let abi_version = manifest.get("abiVersion").and_then(Value::as_u64).unwrap_or(2) as u32;
    let mut exports = Vec::new();
    if let Some(Value::Object(procedures)) = manifest.get("procedures") {
        for (name, kind) in procedures {
            if kind.as_str() == Some("module") {
                exports.push(name.clone());
            }
        }
    }
    let env = ctx.resolved_environment()?;
    let token = workflow_bearer_token(ctx, &env).await?;
    let url =
        format!("{}/v1/api/functions/modules/{module}/activate", env.url.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(ctx.cli_config.resolved_server().timeout))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to create HTTP client: {error}"))
        })?
        .post(url)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "artifact": artifact,
            "contractHash": contract_hash,
            "abiVersion": abi_version,
            "exports": exports,
        }))
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("function activate failed: {error}"))
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CLIError::ConfigurationError(format!(
            "function activate failed ({status}): {body}"
        )));
    }
    ctx.output().status(format!("activated function module {module}"));
    Ok(())
}

pub async fn rollback_function(ctx: &WorkflowContext, revision: &str) -> Result<()> {
    if revision.trim().is_empty() {
        return Err(CLIError::ConfigurationError(
            "function rollback requires a revision id".into(),
        ));
    }
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let module = ctx.config.functions.module.as_str();
    output.status(format!("CAS rolling back function module {module} to revision {revision}"));
    let token = workflow_bearer_token(ctx, &env).await?;
    let url =
        format!("{}/v1/api/functions/modules/{module}/rollback", env.url.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(ctx.cli_config.resolved_server().timeout))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to create HTTP client: {error}"))
        })?
        .post(url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "revisionId": revision }))
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("function rollback failed: {error}"))
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CLIError::ConfigurationError(format!(
            "function rollback failed ({status}): {body}"
        )));
    }
    output.status(format!("rolled back to revision {revision} (pointer swap, no rebuild)"));
    Ok(())
}

pub async fn show_function_logs(ctx: &WorkflowContext, procedure: Option<&str>) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    output.status("procedure logs");
    let sql = match procedure {
        Some(name) => {
            let escaped = name.replace('\'', "''");
            format!(
                "SELECT timestamp, procedure_id, outcome, channel, level, error_code, message, \
                 duration_ms FROM system.procedure_logs WHERE procedure_id LIKE '%{escaped}%' \
                 ORDER BY timestamp DESC LIMIT 50"
            )
        },
        None => "SELECT timestamp, procedure_id, outcome, channel, level, error_code, message, \
                 duration_ms FROM system.procedure_logs ORDER BY timestamp DESC LIMIT 50"
            .to_string(),
    };
    execute_and_print(ctx, &client, &sql, Some(env.namespace.as_str()), "functions logs").await
}

pub async fn show_function_runtime(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    let namespace = Some(env.namespace.as_str());
    output.status("function memory");
    execute_and_print(
        ctx,
        &client,
        "SELECT metric_name, metric_value FROM system.stats WHERE metric_name LIKE \
         'function_memory%' OR metric_name LIKE 'function_instances%' ORDER BY metric_name",
        namespace,
        "functions runtime",
    )
    .await?;
    output.status("resident isolates");
    execute_and_print(
        ctx,
        &client,
        "SELECT instance_id, worker, module_id, revision_id, state, reserved_bytes, \
         used_heap_bytes, invocations FROM system.module_instances ORDER BY worker, instance_id",
        namespace,
        "functions runtime",
    )
    .await?;
    output.status("in-flight root calls");
    execute_and_print(
        ctx,
        &client,
        "SELECT execution_id, request_id, procedure_id, revision_id, actor, origin, started_at, \
         depth FROM system.active_procedure_runs ORDER BY started_at",
        namespace,
        "functions runtime",
    )
    .await
}

async fn workflow_bearer_token(
    ctx: &WorkflowContext,
    env: &crate::workflow::project::resolve::ResolvedEnvironment,
) -> Result<String> {
    match resolve_workflow_auth_provider(ctx, env)? {
        AuthProvider::JwtToken(token) => Ok(token),
        AuthProvider::BasicAuth(user, password) => {
            let login =
                login_with_credentials(&env.url, &user, &password).await.map_err(|error| {
                    CLIError::ConfigurationError(format!("function module auth failed: {error}"))
                })?;
            Ok(login.access_token)
        },
        AuthProvider::None => Err(CLIError::ConfigurationError(
            "function activate/rollback requires a saved CLI authentication profile".into(),
        )),
    }
}

pub fn override_function(ctx: &WorkflowContext, procedure: &str) -> Result<()> {
    let (namespace, name) = procedure.split_once('.').ok_or_else(|| {
        CLIError::ConfigurationError(
            "functions override requires namespace.name (example: api.health)".into(),
        )
    })?;
    let key = format!("{namespace}.{name}");
    if let Ok((snapshot, _)) = compile_project_contract(&ctx.project_root, &ctx.config) {
        let bindings = discover_procedure_bindings(&ctx.project_root, &snapshot)?;
        if let Some(existing) = bindings.iter().find(|binding| binding.routine_id == key) {
            return Err(CLIError::ConfigurationError(format!(
                "procedure {key} is already bound as '{}' in {}",
                existing.export_name,
                existing.source_path.display()
            )));
        }
    }
    let path = procedure_impl_path(&ctx.project_root, namespace, name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CLIError::FileError(format!("failed to create '{}': {error}", parent.display()))
        })?;
    }
    if path.exists() {
        return Err(CLIError::ConfigurationError(format!(
            "override already exists at {}",
            path.display()
        )));
    }
    let body = seed_override_body(ctx, namespace, name);
    fs::write(&path, body).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", path.display()))
    })?;
    ctx.output().status(format!("scaffolded {}", path.display()));
    Ok(())
}

fn seed_override_body(ctx: &WorkflowContext, namespace: &str, name: &str) -> String {
    let Ok((snapshot, _)) = compile_project_contract(&ctx.project_root, &ctx.config) else {
        return implemented_procedure_source(namespace, name, None);
    };
    let key = format!("{namespace}.{name}");
    let inline = snapshot.routines.get(&key).and_then(|routine| routine.body.clone());
    implemented_procedure_source(namespace, name, inline.as_deref())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn rejects_node_builtin_and_napi_packages() {
        let temp = TempDir::new().unwrap();
        let functions = temp.path().join("functions");
        fs::create_dir_all(&functions).unwrap();
        fs::write(functions.join("package.json"), r#"{"dependencies":{"fs":"1.0.0"}}"#).unwrap();
        let err = validate_function_packages(temp.path()).unwrap_err();
        assert!(err.to_string().contains("fs"), "{err}");

        fs::write(
            functions.join("package.json"),
            r#"{"optionalDependencies":{"node-addon-api":"8.0.0"}}"#,
        )
        .unwrap();
        let err = validate_function_packages(temp.path()).unwrap_err();
        assert!(err.to_string().contains("node-addon-api"), "{err}");
    }

    #[test]
    fn allows_pure_javascript_packages() {
        let temp = TempDir::new().unwrap();
        let functions = temp.path().join("functions");
        fs::create_dir_all(&functions).unwrap();
        fs::write(functions.join("package.json"), r#"{"dependencies":{"zod":"3.23.8"}}"#).unwrap();
        validate_function_packages(temp.path()).unwrap();
    }

    #[test]
    fn rejects_native_node_addon_files() {
        let temp = TempDir::new().unwrap();
        let functions = temp.path().join("functions");
        fs::create_dir_all(functions.join("src")).unwrap();
        fs::write(functions.join("src/addon.node"), b"native").unwrap();
        let err = validate_function_packages(temp.path()).unwrap_err();
        assert!(err.to_string().contains("native addon"), "{err}");
    }

    #[test]
    fn hashes_lockfile_when_present() {
        let temp = TempDir::new().unwrap();
        let functions = temp.path().join("functions");
        fs::create_dir_all(&functions).unwrap();
        fs::write(functions.join("package-lock.json"), "{}").unwrap();
        let hash = lockfile_hash(temp.path()).unwrap();
        assert_eq!(hash.len(), 64);
    }

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
    fn esbuild_node_paths_uses_project_node_modules() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("functions/src/api")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::create_dir_all(root.join("functions/node_modules")).unwrap();
        fs::write(root.join("kalam.toml"), "name = \"demo\"\n").unwrap();
        let source = root.join("functions/src/api/health.ts");
        fs::write(&source, "export default async () => {}\n").unwrap();
        let joined = esbuild_node_paths(&source).unwrap();
        let root_modules = root.join("node_modules").canonicalize().unwrap();
        let functions_modules = root.join("functions/node_modules").canonicalize().unwrap();
        assert!(joined.contains(root_modules.to_str().unwrap()), "{joined}");
        assert!(joined.contains(functions_modules.to_str().unwrap()), "{joined}");
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
    fn override_scaffolds_named_builder() {
        let temp = TempDir::new().unwrap();
        let ctx = crate::workflow::test_support::test_workflow_context(temp.path());
        override_function(&ctx, "api.health").unwrap();
        let path = temp.path().join("functions/src/api/health.ts");
        let source = fs::read_to_string(&path).unwrap();
        assert!(source.contains("procedure.api.health("));
        assert!(source.contains("export const health"));
        let err = override_function(&ctx, "api.health").unwrap_err();
        assert!(
            err.to_string().contains("already bound") || err.to_string().contains("already exists"),
            "{err}"
        );
    }

    #[test]
    fn empty_registry_emits_kalam_invoke() {
        let js = empty_module_artifact();
        assert!(js.contains("function kalamInvoke"));
        assert!(js.contains("const procedures = {}"));
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

    #[test]
    fn esbuild_bundles_named_registry_to_javascript() {
        let esbuild = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/chat-with-ai/node_modules/esbuild/bin/esbuild");
        if !esbuild.is_file() {
            return;
        }
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/chat-with-ai");
        let registry = example.join("functions/src/generated/registry.ts");
        if !registry.is_file() {
            return;
        }
        let js = bundle_registry(&esbuild, &registry).unwrap();
        assert!(js.contains("kalamInvoke"));
        assert!(js.contains("chat_demo.join_room"));
        assert!(js.contains("chat_demo.send_message"));
        assert!(js.contains("chat_demo.on_user_message"));
        assert!(js.contains("AI reply"));
        assert_eq!(
            js.matches("drizzle:entityKind").count(),
            1,
            "shared deps should be bundled once, got {} copies in {} bytes",
            js.matches("drizzle:entityKind").count(),
            js.len()
        );
        assert!(
            js.len() < 250_000,
            "minified single-graph artifact should be well under the old 3x IIFE size, got {}",
            js.len()
        );
        assert!(!js.lines().any(|line| line.trim_start().starts_with("import ")));
        assert!(!js.contains("export default"));
        assert!(!js.contains("type ChatDemo"));
        assert!(!js.contains("defineProcedure<"));
    }
}
