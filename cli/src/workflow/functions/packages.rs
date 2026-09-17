use std::{collections::BTreeSet, fs, path::Path};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{CLIError, Result};

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

#[cfg(test)]
mod tests {
    use std::fs;

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
}
