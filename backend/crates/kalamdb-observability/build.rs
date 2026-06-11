// Crate-local build script for crates.io packaging.
// Captures build metadata and optionally compiles the glibc isoc23 shim on linux-gnu.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let repo_root = find_repo_root(&manifest_dir).unwrap_or_else(|| manifest_dir.clone());

    build_isoc23_glibc_shim_if_needed(&manifest_dir, &repo_root);

    let fallback = read_version_toml(&repo_root.join("version.toml"));

    let commit_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.0.clone());

    let build_date = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.1.clone());

    println!("cargo:rustc-env=GIT_COMMIT_HASH={commit_hash}");
    println!("cargo:rustc-env=BUILD_DATE={build_date}");
    println!("cargo:rustc-env=GIT_BRANCH={branch}");

    let git_head = repo_root.join(".git").join("HEAD");
    let git_heads_dir = repo_root.join(".git").join("refs").join("heads");
    if git_head.exists() {
        println!("cargo:rerun-if-changed={}", git_head.display());
    }
    if git_heads_dir.exists() {
        println!("cargo:rerun-if-changed={}", git_heads_dir.display());
    }
    let version_toml = repo_root.join("version.toml");
    if version_toml.exists() {
        println!("cargo:rerun-if-changed={}", version_toml.display());
    }
}

fn build_isoc23_glibc_shim_if_needed(manifest_dir: &Path, repo_root: &Path) {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os != "linux" || target_env != "gnu" {
        return;
    }

    let packaged_shim = manifest_dir.join("build").join("isoc23_shim.c");
    let in_tree_shim = repo_root.join("backend").join("build").join("isoc23_shim.c");
    let shim_path = if packaged_shim.exists() {
        packaged_shim
    } else {
        in_tree_shim
    };

    if !shim_path.exists() {
        println!(
            "cargo:warning=Skipping isoc23 glibc shim; source file not found at {}",
            shim_path.display()
        );
        return;
    }

    println!("cargo:rerun-if-changed={}", shim_path.display());
    cc::Build::new().file(&shim_path).compile("kalamdb_isoc23_shim");
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join("version.toml").exists() || ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn read_version_toml(path: &Path) -> (String, String, String) {
    let default = ("unknown".to_string(), "unknown".to_string(), "unknown".to_string());

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return default,
    };

    let mut commit = "unknown".to_string();
    let mut branch = "unknown".to_string();
    let mut build_date = "unknown".to_string();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("git_commit_hash") {
            if let Some(val) = extract_toml_value(line) {
                commit = val;
            }
        } else if line.starts_with("git_branch") {
            if let Some(val) = extract_toml_value(line) {
                branch = val;
            }
        } else if line.starts_with("build_date") {
            if let Some(val) = extract_toml_value(line) {
                build_date = val;
            }
        }
    }

    (commit, branch, build_date)
}

fn extract_toml_value(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.splitn(2, '=').collect();
    if parts.len() == 2 {
        let val = parts[1].trim().trim_matches('"');
        if !val.is_empty() && val != "unknown" {
            return Some(val.to_string());
        }
    }
    None
}
