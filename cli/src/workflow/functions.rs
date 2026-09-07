//! `kalam functions` build/status/rollback/logs.

use std::{collections::BTreeSet, fs, path::Path, process::Command, time::Duration};

use kalam_client::AuthProvider;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    error::{CLIError, Result},
    workflow::{
        auth::{login_with_credentials, resolve_workflow_auth_provider},
        generate_schema,
        sql::{build_workflow_client, execute_single_statement},
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
    let mut procedures = Vec::new();
    let src = ctx.project_root.join("functions/src");
    if src.exists() {
        let mut stack = vec![src.clone()];
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
                if path.extension().and_then(|ext| ext.to_str()) != Some("ts") {
                    continue;
                }
                let rel = path.strip_prefix(&src).unwrap_or(&path);
                let Some(stem) = rel.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let schema =
                    rel.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
                if schema.is_empty() {
                    continue;
                }
                let text = fs::read_to_string(&path).map_err(|error| {
                    CLIError::FileError(format!("failed to read '{}': {error}", path.display()))
                })?;
                procedures.push((format!("{schema}.{stem}"), text));
            }
        }
    }
    let source = bundle_procedures(&procedures)?;
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

fn bundle_procedures(procedures: &[(String, String)]) -> Result<String> {
    let mut out = String::from(
        "function kalamInvoke(name, args) {\n  const ctx = globalThis.__kalamCtx;\n  const input \
         = args.length === 1 ? args[0] : Array.from(args);\n  const fn = procedures[name];\n  if \
         (typeof fn !== \"function\") {\n    throw new Error(\"missing export \" + name);\n  }\n  \
         return fn(ctx, input);\n}\nconst procedures = {\n",
    );
    for (name, source) in procedures {
        let handler = strip_ts_default_export(source)?;
        out.push_str("  \"");
        out.push_str(name);
        out.push_str("\": (function() { return ");
        out.push_str(&handler);
        out.push_str("; })(),\n");
    }
    out.push_str("};\n");
    Ok(out)
}

fn strip_ts_default_export(source: &str) -> Result<String> {
    let mut without_imports = String::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("import ") {
            continue;
        }
        without_imports.push_str(line);
        without_imports.push('\n');
    }
    let Some(index) = without_imports.find("export default") else {
        return Err(CLIError::ConfigurationError(
            "procedure source is missing export default".into(),
        ));
    };
    let mut expr = without_imports[index + "export default".len()..].trim().to_string();
    if expr.ends_with(';') {
        expr.pop();
    }
    let expr = strip_simple_type_annotations(&expr);
    Ok(expr)
}

fn strip_simple_type_annotations(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len()
                && (chars[j].is_ascii_alphabetic() || chars[j] == '{' || chars[j] == '(')
            {
                while j < chars.len() && !matches!(chars[j], ',' | ')' | '{' | '=' | ';' | '\n') {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn write_build_manifest(ctx: &WorkflowContext) -> Result<()> {
    let (snapshot, contract_hash) =
        crate::workflow::schema::compile_project_contract(&ctx.project_root, &ctx.config)?;
    validate_registry_exports(&ctx.project_root, &snapshot)?;
    let mut procedures = serde_json::Map::new();
    for routine in snapshot.routines.values() {
        let kind = if routine.body.is_some() {
            "inline"
        } else {
            "module"
        };
        procedures.insert(routine.routine_id.to_string(), Value::String(kind.to_string()));
    }
    let registry = ctx.project_root.join("functions/.kalam/generated/registry.ts");
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

fn validate_registry_exports(
    project_root: &Path,
    snapshot: &kalamdb_sql::contracts::ContractSnapshot,
) -> Result<()> {
    for routine in snapshot.routines.values() {
        if routine.body.is_some() {
            continue;
        }
        let path = project_root
            .join("functions/src")
            .join(&routine.schema)
            .join(format!("{}.ts", routine.name));
        if !path.exists() {
            return Err(CLIError::ConfigurationError(format!(
                "missing export for project-backed procedure {}",
                routine.routine_id
            )));
        }
        let text = fs::read_to_string(&path).map_err(|error| {
            CLIError::FileError(format!("failed to read '{}': {error}", path.display()))
        })?;
        if !text.contains("export default") {
            return Err(CLIError::ConfigurationError(format!(
                "missing export for project-backed procedure {}",
                routine.routine_id
            )));
        }
    }
    let src = project_root.join("functions/src");
    if !src.exists() {
        return Ok(());
    }
    let mut stack = vec![src];
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
            if path.extension().and_then(|ext| ext.to_str()) != Some("ts") {
                continue;
            }
            let rel = path.strip_prefix(project_root.join("functions/src")).unwrap_or(&path);
            let Some(stem) = rel.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let schema =
                rel.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
            if schema.is_empty() {
                continue;
            }
            let qualified = format!("{schema}.{stem}");
            if !snapshot.routines.contains_key(&qualified)
                && !snapshot
                    .routines
                    .values()
                    .any(|routine| routine.schema == schema && routine.name == stem)
            {
                return Err(CLIError::ConfigurationError(format!(
                    "unknown export {qualified} has no SQL routine"
                )));
            }
        }
    }
    Ok(())
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
    output.status("listing active function module and routines");
    execute_single_statement(
        &client,
        "SELECT module_id, runtime, active_revision_id, contract_hash, abi_version FROM \
         system.function_modules ORDER BY module_id",
        Some(env.namespace.as_str()),
        "functions status",
    )
    .await?;
    execute_single_statement(
        &client,
        "SELECT routine_id, language, security FROM system.routines ORDER BY routine_id",
        Some(env.namespace.as_str()),
        "functions status",
    )
    .await
}

pub async fn show_function_revisions(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    output.status("listing system.function_revisions");
    execute_single_statement(
        &client,
        "SELECT module_id, revision_id, artifact_id, contract_hash, abi_version, created_at FROM \
         system.function_revisions ORDER BY created_at DESC",
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
    output.status("listing structured function errors from system.function_errors");
    let sql = match procedure {
        Some(name) => {
            let escaped = name.replace('\'', "''");
            format!(
                "SELECT execution_id, request_id, routine_id, actor, origin, code, message, \
                 recorded_at FROM system.function_errors WHERE routine_id LIKE '%{escaped}%' \
                 ORDER BY recorded_at DESC LIMIT 50"
            )
        },
        None => "SELECT execution_id, request_id, routine_id, actor, origin, code, message, \
                 recorded_at FROM system.function_errors ORDER BY recorded_at DESC LIMIT 50"
            .to_string(),
    };
    execute_single_statement(&client, &sql, Some(env.namespace.as_str()), "functions logs").await
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
    let (schema, name) = procedure.split_once('.').ok_or_else(|| {
        CLIError::ConfigurationError(
            "functions override requires schema.name (example: api.health)".into(),
        )
    })?;
    let dir = ctx.project_root.join("functions/src").join(schema);
    fs::create_dir_all(&dir).map_err(|error| {
        CLIError::FileError(format!("failed to create '{}': {error}", dir.display()))
    })?;
    let path = dir.join(format!("{name}.ts"));
    if path.exists() {
        return Err(CLIError::ConfigurationError(format!(
            "override already exists at {}",
            path.display()
        )));
    }
    let body = seed_override_body(ctx, schema, name);
    fs::write(&path, body).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", path.display()))
    })?;
    ctx.output().status(format!("scaffolded {}", path.display()));
    Ok(())
}

fn seed_override_body(ctx: &WorkflowContext, schema: &str, name: &str) -> String {
    let Ok((snapshot, _)) =
        crate::workflow::schema::compile_project_contract(&ctx.project_root, &ctx.config)
    else {
        return default_override_source(schema, name, None);
    };
    let key = format!("{schema}.{name}");
    let inline = snapshot.routines.get(&key).and_then(|routine| routine.body.clone());
    default_override_source(schema, name, inline.as_deref())
}

fn default_override_source(schema: &str, name: &str, inline: Option<&str>) -> String {
    match inline {
        Some(body) if !body.trim().is_empty() => {
            format!(
                "/** Override for {schema}.{name}. Seeded from the inline SQL body. */\nexport \
                 default async (ctx: KalamCtx, input: unknown) => {{\n{body}\n}};\n"
            )
        },
        _ => format!(
            "/** Override for {schema}.{name}. */\nexport default async (ctx: KalamCtx, input: \
             unknown) => {{\n  return input;\n}};\n"
        ),
    }
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
    fn rejects_unknown_and_missing_exports() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("functions/src/api")).unwrap();
        fs::write(root.join("functions/src/api/orphan.ts"), "export default async () => {}")
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
            },
        );
        let err = validate_registry_exports(root, &snapshot).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("missing export") || message.contains("unknown export"),
            "{message}"
        );
    }

    #[test]
    fn override_scaffolds_schema_proc_file() {
        let temp = TempDir::new().unwrap();
        let ctx = crate::workflow::test_support::test_workflow_context(temp.path());
        override_function(&ctx, "api.health").unwrap();
        let path = temp.path().join("functions/src/api/health.ts");
        let source = fs::read_to_string(&path).unwrap();
        assert!(source.contains("export default"));
        let err = override_function(&ctx, "api.health").unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[test]
    fn bundles_typescript_default_export_into_kalam_invoke() {
        let source = "import type { KalamCtx } from \"./runtime\";\nexport default async (ctx: \
                      KalamCtx, input: unknown) => {\n  return input;\n};\n";
        let js = bundle_procedures(&[("api.health".into(), source.into())]).unwrap();
        assert!(js.contains("function kalamInvoke"));
        assert!(js.contains("api.health"));
        assert!(!js.contains("KalamCtx"));
        assert!(js.contains("return input"));
    }
}
