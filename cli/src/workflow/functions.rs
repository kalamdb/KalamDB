//! `kalam functions` build/status/rollback/logs.

use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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

fn is_typescript_declaration(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".d.ts"))
}

fn is_procedure_typescript(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("ts") && !is_typescript_declaration(path)
}

fn is_generated_functions_dir(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("generated")
}

fn is_generated_functions_rel(rel: &Path) -> bool {
    rel.components().any(|component| component.as_os_str() == "generated")
}

fn visit_procedure_sources(
    project_root: &Path,
    mut visit: impl FnMut(&Path, &str, &str) -> Result<()>,
) -> Result<()> {
    let src = project_root.join("functions/src");
    if !src.exists() {
        return Ok(());
    }
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if is_generated_functions_dir(&path) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !is_procedure_typescript(&path) {
                continue;
            }
            let rel = path.strip_prefix(&src).unwrap_or(&path);
            if is_generated_functions_rel(rel) {
                continue;
            }
            let Some(stem) = rel.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let namespace =
                rel.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
            if namespace.is_empty() {
                continue;
            }
            visit(&path, namespace, stem)?;
        }
    }
    Ok(())
}

fn write_module_artifact(ctx: &WorkflowContext) -> Result<()> {
    let mut procedures = Vec::new();
    visit_procedure_sources(&ctx.project_root, |path, namespace, stem| {
        let text = fs::read_to_string(path).map_err(|error| {
            CLIError::FileError(format!("failed to read '{}': {error}", path.display()))
        })?;
        procedures.push((format!("{namespace}.{stem}"), text, path.to_path_buf()));
        Ok(())
    })?;
    let source =
        bundle_procedure_files(&procedures, find_esbuild_bin(&ctx.project_root).as_deref())?;
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

fn esbuild_bundle_file(esbuild_bin: &Path, path: &Path) -> Result<String> {
    let sourcefile = path.to_str().ok_or_else(|| {
        CLIError::FileError(format!("procedure path is not utf-8: {}", path.display()))
    })?;
    // esbuild `--outfile=-` writes a file named `-`, not stdout.
    let mut outfile = std::env::temp_dir();
    outfile.push(format!(
        "kalam-fn-{}-{}.js",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
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
        "--format=esm",
        "--platform=neutral",
        "--target=es2022",
        "--log-level=error",
        "--legal-comments=none",
        &format!("--outfile={outfile_arg}"),
    ]);
    if let Some(node_paths) = esbuild_node_paths(path) {
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
    let source = fs::read_to_string(&outfile)
        .map_err(|error| CLIError::FileError(format!("failed to read esbuild output: {error}")))?;
    let _ = fs::remove_file(&outfile);
    Ok(source)
}

fn esbuild_ts_to_esm(esbuild_bin: &Path, source: &str, sourcefile: &str) -> Result<String> {
    let mut child = Command::new(esbuild_bin)
        .args([
            "--loader=ts",
            "--format=esm",
            "--platform=neutral",
            "--target=es2022",
            "--log-level=error",
            &format!("--sourcefile={sourcefile}"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CLIError::FileError(format!("failed to start esbuild: {error}")))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| CLIError::FileError("failed to open esbuild stdin".into()))?;
        stdin.write_all(source.as_bytes()).map_err(|error| {
            CLIError::FileError(format!("failed to write esbuild stdin: {error}"))
        })?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| CLIError::FileError(format!("failed to wait for esbuild: {error}")))?;
    if !output.status.success() {
        return Err(CLIError::ConfigurationError(format!(
            "esbuild failed for {sourcefile}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| CLIError::FileError(format!("esbuild output was not utf-8: {error}")))
}

fn esm_default_export_name(source: &str) -> Option<&str> {
    let marker = " as default";
    let index = source.find(marker)?;
    let before = source[..index].trim_end();
    let ident = before
        .rsplit(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .next()
        .unwrap_or("");
    if ident.is_empty() {
        None
    } else {
        Some(ident)
    }
}

fn strip_import_declarations(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let at_statement_start = i == 0 || chars[i - 1] == '\n' || chars[i - 1] == ';';
        if at_statement_start && starts_with_word(&chars, i, "import") {
            let mut j = i + 6;
            let mut depth = 0i32;
            while j < chars.len() {
                match chars[j] {
                    '{' | '(' | '[' => depth += 1,
                    '}' | ')' | ']' => depth -= 1,
                    ';' if depth <= 0 => {
                        j += 1;
                        break;
                    },
                    _ => {},
                }
                j += 1;
            }
            if j < chars.len() && chars[j] == '\n' {
                j += 1;
            }
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn starts_with_word(chars: &[char], index: usize, word: &str) -> bool {
    let word_chars: Vec<char> = word.chars().collect();
    if index + word_chars.len() > chars.len() {
        return false;
    }
    if chars[index..index + word_chars.len()] != word_chars {
        return false;
    }
    let after = index + word_chars.len();
    after == chars.len() || !(chars[after].is_ascii_alphanumeric() || chars[after] == '_')
}

fn wrap_esbuild_esm(esm: &str) -> Result<String> {
    let without_imports = strip_import_declarations(esm);
    if without_imports.contains("export default") {
        return Ok(without_imports.replace("export default", "return"));
    }
    let default_name = esm_default_export_name(&without_imports).ok_or_else(|| {
        CLIError::ConfigurationError("esbuild output is missing a default export".into())
    })?;
    let mut body = strip_named_export_lists(&without_imports);
    body.push_str("\nreturn ");
    body.push_str(default_name);
    body.push_str(";\n");
    Ok(body)
}

fn looks_like_typed_typescript(source: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("type ")
            || trimmed.starts_with("export type ")
            || trimmed.starts_with("interface ")
            || (trimmed.starts_with("import ") && trimmed.contains('{') && !trimmed.contains('}'))
    })
}

fn prepare_procedure_source(
    source: &str,
    esbuild: Option<&Path>,
    sourcefile: &str,
    source_path: Option<&Path>,
) -> Result<String> {
    if let Some(bin) = esbuild {
        let esm = if let Some(path) = source_path {
            esbuild_bundle_file(bin, path)?
        } else {
            esbuild_ts_to_esm(bin, source, sourcefile)?
        };
        return wrap_esbuild_esm(&esm);
    }
    if looks_like_typed_typescript(source) {
        return Err(CLIError::ConfigurationError(
            "functions TypeScript with types requires esbuild; run npm install in the project \
             root (Vite includes it) or add esbuild to functions/package.json"
                .into(),
        ));
    }
    strip_ts_default_export(source)
}

#[cfg(test)]
fn bundle_procedures(procedures: &[(String, String)]) -> Result<String> {
    bundle_procedures_with(procedures, None)
}

fn bundle_procedure_files(
    procedures: &[(String, String, PathBuf)],
    esbuild: Option<&Path>,
) -> Result<String> {
    bundle_named_sources(
        procedures
            .iter()
            .map(|(name, source, path)| (name.as_str(), source.as_str(), Some(path.as_path()))),
        esbuild,
    )
}

#[cfg(test)]
fn bundle_procedures_with(
    procedures: &[(String, String)],
    esbuild: Option<&Path>,
) -> Result<String> {
    bundle_named_sources(
        procedures.iter().map(|(name, source)| (name.as_str(), source.as_str(), None)),
        esbuild,
    )
}

fn bundle_named_sources<'a>(
    procedures: impl IntoIterator<Item = (&'a str, &'a str, Option<&'a Path>)>,
    esbuild: Option<&Path>,
) -> Result<String> {
    let mut out = String::from(
        "function defineProcedure(handler) {\n  return handler;\n}\nfunction kalamInvoke(name, \
         args) {\n  const ctx = globalThis.__kalamCtx;\n  const input = args.length === 1 ? \
         args[0] : Array.from(args);\n  const fn = procedures[name];\n  if (typeof fn !== \
         \"function\") {\n    throw new Error(\"missing export \" + name);\n  }\n  return fn(ctx, \
         input);\n}\nconst procedures = {\n",
    );
    for (name, source, source_path) in procedures {
        let sourcefile = format!("{}.ts", name.replace('.', "_"));
        let handler = prepare_procedure_source(source, esbuild, &sourcefile, source_path)?;
        out.push_str("  \"");
        out.push_str(name);
        out.push_str("\": (function() {\n");
        out.push_str(&handler);
        out.push_str("\n})(),\n");
    }
    out.push_str("};\n");
    Ok(out)
}

fn strip_ts_default_export(source: &str) -> Result<String> {
    let without_imports = strip_import_declarations(source);
    if !without_imports.contains("export default") {
        return Err(CLIError::ConfigurationError(
            "procedure source is missing export default".into(),
        ));
    }
    let mut body = without_imports.replace("export default", "return");
    body = body.replace("export async function", "async function");
    body = body.replace("export function", "function");
    body = body.replace("export const", "const");
    body = body.replace("export let", "let");
    body = body.replace("export var", "var");
    let body = strip_named_export_lists(&body);
    let body = strip_simple_type_annotations(&body);
    Ok(strip_simple_generics(&body))
}

fn strip_named_export_lists(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].starts_with(&['e', 'x', 'p', 'o', 'r', 't']) {
            let mut j = i + 6;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '{' {
                let mut depth = 1;
                j += 1;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {},
                    }
                    j += 1;
                }
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ';' {
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

fn strip_simple_generics(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i > 0 && (chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_')
        {
            let mut depth = 1;
            i += 1;
            while i < chars.len() && depth > 0 {
                match chars[i] {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    _ => {},
                }
                i += 1;
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
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
    visit_procedure_sources(project_root, |_path, namespace, stem| {
        let qualified = format!("{namespace}.{stem}");
        if !snapshot.routines.contains_key(&qualified)
            && !snapshot
                .routines
                .values()
                .any(|routine| routine.schema == namespace && routine.name == stem)
        {
            return Err(CLIError::ConfigurationError(format!(
                "unknown export {qualified} has no SQL routine"
            )));
        }
        Ok(())
    })
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
    let (namespace, name) = procedure.split_once('.').ok_or_else(|| {
        CLIError::ConfigurationError(
            "functions override requires namespace.name (example: api.health)".into(),
        )
    })?;
    let dir = ctx.project_root.join("functions/src").join(namespace);
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
    let body = seed_override_body(ctx, namespace, name);
    fs::write(&path, body).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", path.display()))
    })?;
    ctx.output().status(format!("scaffolded {}", path.display()));
    Ok(())
}

fn seed_override_body(ctx: &WorkflowContext, namespace: &str, name: &str) -> String {
    let Ok((snapshot, _)) =
        crate::workflow::schema::compile_project_contract(&ctx.project_root, &ctx.config)
    else {
        return default_override_source(namespace, name, None);
    };
    let key = format!("{namespace}.{name}");
    let inline = snapshot.routines.get(&key).and_then(|routine| routine.body.clone());
    default_override_source(namespace, name, inline.as_deref())
}

fn default_override_source(namespace: &str, name: &str, inline: Option<&str>) -> String {
    match inline {
        Some(body) if !body.trim().is_empty() => {
            format!(
                "/** Override for {namespace}.{name}. Seeded from the inline SQL body. */\nexport \
                 default async (ctx: KalamCtx, input: unknown) => {{\n{body}\n}};\n"
            )
        },
        _ => format!(
            "/** Override for {namespace}.{name}. */\nexport default async (ctx: KalamCtx, input: \
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
        assert!(
            joined.contains(root_modules.to_str().unwrap()),
            "{joined}"
        );
        assert!(
            joined.contains(functions_modules.to_str().unwrap()),
            "{joined}"
        );
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
    fn override_scaffolds_namespace_proc_file() {
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
        assert!(js.contains("function defineProcedure"));
        assert!(js.contains("return input"));
    }

    #[test]
    fn bundles_define_procedure_generics() {
        let source = "import { defineProcedure, type ChatSend } from \
                      \"../generated/contracts\";\nexport default defineProcedure<ChatSend>(\n  \
                      async (ctx, input) => {\n    return input;\n  },\n);\n";
        let js = bundle_procedures(&[("chat.send_message".into(), source.into())]).unwrap();
        assert!(js.contains("defineProcedure("));
        assert!(!js.contains("ChatSend"));
        assert!(!js.contains("defineProcedure<"));
    }

    #[test]
    fn bundles_file_level_helpers_with_default_export() {
        let source = "function buildReply(content) {\n  return 'AI reply: ' + content;\n}\nexport \
                      default async (ctx, input) => buildReply(input);\n";
        let js = bundle_procedures(&[("chat.reply".into(), source.into())]).unwrap();
        assert!(js.contains("function buildReply"));
        assert!(js.contains("return async (ctx, input) => buildReply(input)"));
    }

    #[test]
    fn typed_multiline_import_requires_esbuild_without_bin() {
        let source =
            include_str!("../../../examples/chat-with-ai/functions/src/chat_demo/join_room.ts");
        let err = bundle_procedures(&[("chat_demo.join_room".into(), source.into())]).unwrap_err();
        assert!(
            err.to_string().contains("esbuild"),
            "typed procedures should ask for esbuild: {err}"
        );
    }

    #[test]
    fn esbuild_bundles_chat_procedures_to_javascript() {
        let esbuild = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/chat-with-ai/node_modules/esbuild/bin/esbuild");
        if !esbuild.is_file() {
            return;
        }
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/chat-with-ai");
        let join_room = example.join("functions/src/chat_demo/join_room.ts");
        let send_message = example.join("functions/src/chat_demo/send_message.ts");
        let on_user_message = example.join("functions/src/chat_demo/on_user_message.ts");
        if !join_room.is_file() {
            return;
        }
        let js = bundle_procedure_files(
            &[
                ("chat_demo.join_room".into(), fs::read_to_string(&join_room).unwrap(), join_room),
                (
                    "chat_demo.send_message".into(),
                    fs::read_to_string(&send_message).unwrap(),
                    send_message,
                ),
                (
                    "chat_demo.on_user_message".into(),
                    fs::read_to_string(&on_user_message).unwrap(),
                    on_user_message,
                ),
            ],
            Some(&esbuild),
        )
        .unwrap();
        assert!(js.contains("function kalamInvoke"));
        assert!(js.contains("function buildReply"));
        assert!(js.contains("chatDemoRooms"));
        assert!(js.contains("chatDemoMessages"));
        assert!(js.contains("bindFunctionOrm") || js.contains("kalamFunctionDb"));
        assert!(js.contains("return join_room_default"));
        assert!(js.contains("return send_message_default"));
        assert!(js.contains("return on_user_message_default"));
        assert!(!js.lines().any(|line| line.trim_start().starts_with("import ")));
        assert!(!js.contains("export default"));
        assert!(!js.contains("\nexport {"));
        assert!(!js.contains("type ChatDemo"));
        assert!(!js.contains("defineProcedure<"));
    }
}
