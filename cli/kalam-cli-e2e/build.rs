// kalam-cli-e2e path-includes `cli/src/args.rs` for clap parsing tests, so this crate
// must provide the same compile-time build metadata env vars as `kalam-cli/build.rs`.

use std::{env, fs, process::Command};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let fallback = read_version_toml(&format!("{manifest_dir}/../../version.toml"));

    let commit_hash =
        git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| fallback.0.clone());
    let branch =
        git_output(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| fallback.1.clone());
    let build_date = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    println!("cargo:rustc-env=GIT_COMMIT_HASH={commit_hash}");
    println!("cargo:rustc-env=GIT_BRANCH={branch}");
    println!("cargo:rustc-env=BUILD_DATE={build_date}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads/");
    println!("cargo:rerun-if-changed=../../version.toml");
    println!("cargo:rerun-if-changed=../src/args.rs");
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn read_version_toml(path: &str) -> (String, String, String) {
    let default = ("unknown".to_string(), "unknown".to_string(), "unknown".to_string());

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return default,
    };

    let mut commit = "unknown".to_string();
    let mut branch = "unknown".to_string();
    let mut build_date = "unknown".to_string();

    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = extract_toml_field(line, "git_commit_hash") {
            commit = value;
        } else if let Some(value) = extract_toml_field(line, "git_branch") {
            branch = value;
        } else if let Some(value) = extract_toml_field(line, "build_date") {
            build_date = value;
        }
    }

    (commit, branch, build_date)
}

fn extract_toml_field(line: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} =");
    if !line.starts_with(&prefix) {
        return None;
    }
    let value = line[prefix.len()..].trim().trim_matches('"');
    if value.is_empty() || value == "unknown" {
        None
    } else {
        Some(value.to_string())
    }
}
