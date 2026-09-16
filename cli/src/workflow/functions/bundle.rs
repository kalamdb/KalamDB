use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    error::{CLIError, Result},
    workflow::project::templates::render_schema_gen_file,
};

pub(super) fn find_esbuild_bin(project_root: &Path) -> Option<PathBuf> {
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

pub(super) fn find_tsc_bin(project_root: &Path) -> Option<PathBuf> {
    for rel in ["functions/node_modules/.bin/tsc", "node_modules/.bin/tsc"] {
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

pub(super) fn esbuild_node_paths(source: &Path) -> Option<String> {
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

fn module_entry_source(registry_import: Option<&str>) -> Result<String> {
    render_schema_gen_file(
        "typescript",
        "functions/src/generated/module_entry.js",
        &serde_json::json!({
            "registry_import": registry_import.unwrap_or(""),
        }),
    )
}

pub(super) fn empty_module_artifact() -> Result<String> {
    module_entry_source(None)
}

pub(super) fn bundle_registry(esbuild_bin: &Path, registry: &Path) -> Result<String> {
    let entry = module_entry_source(Some(&path_as_esbuild_import(registry)?))?;
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

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::TempDir;

    use super::*;

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
    fn empty_registry_emits_kalam_invoke() {
        let js = empty_module_artifact().unwrap();
        assert!(js.contains("function kalamInvoke"));
        assert!(js.contains("const procedures = {}"));
    }

    #[test]
    fn finds_project_root_typescript_for_functions_typecheck() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let tsc_path = root.join("node_modules/.bin/tsc");
        fs::create_dir_all(tsc_path.parent().unwrap()).unwrap();
        fs::write(&tsc_path, b"#!/bin/sh\n").unwrap();

        let tsc = find_tsc_bin(root);
        assert_eq!(tsc.as_deref(), Some(tsc_path.as_path()));
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
        let entity_kind_mentions = js.matches("drizzle:entityKind").count();
        assert!(
            (1..=2).contains(&entity_kind_mentions),
            "shared deps should stay on one esbuild graph; drizzle may mention entityKind once or \
             twice per copy, got {entity_kind_mentions} in {} bytes",
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
