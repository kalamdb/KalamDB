//! Path normalization and PATH lookup for child processes.

use std::{
    env,
    path::{Path, PathBuf},
};

/// Normalize a path for use as a child process working directory or shell argument.
///
/// On Windows, `canonicalize()` returns extended-length paths such as
/// `\\?\C:\project`. `cmd.exe` cannot use those as working directories, so
/// convert them back to regular drive paths. True UNC paths are left as-is so
/// callers can use `pushd`.
pub fn shell_working_directory(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.display().to_string();
        if let Some(rest) = raw.strip_prefix(r"\\?\") {
            if let Some(unc_rest) = rest.strip_prefix("UNC\\") {
                return PathBuf::from(format!(r"\\{unc_rest}"));
            }
            return PathBuf::from(rest);
        }
    }

    path.to_path_buf()
}

/// Resolve an executable name on `PATH`, including Windows `PATHEXT` suffixes.
pub fn resolve_program_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        for candidate in program_candidate_names(name) {
            let path = dir.join(&candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Resolve `node` for schema generation and other JS tooling.
pub fn resolve_node_binary() -> String {
    resolve_program_on_path("node")
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "node".to_string())
}

fn program_candidate_names(name: &str) -> Vec<String> {
    if cfg!(windows)
        && !name.ends_with(".exe")
        && !name.ends_with(".cmd")
        && !name.ends_with(".bat")
    {
        // Prefer real Windows executables over extensionless npm/pnpm/yarn shims.
        return vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
            name.to_string(),
        ];
    }

    vec![name.to_string()]
}

/// Whether `path` must be launched through `cmd.exe` instead of `CreateProcess`.
pub fn program_needs_shell_launch(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "cmd" | "bat"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_program_on_path_finds_cmd_shims_on_windows() {
        if !cfg!(windows) {
            return;
        }
        let path = resolve_program_on_path("cmd");
        assert!(path.is_some(), "cmd should resolve on Windows");
    }

    #[test]
    fn shell_working_directory_strips_extended_windows_prefix() {
        if !cfg!(windows) {
            return;
        }
        let normalized =
            shell_working_directory(Path::new(r"\\?\C:\git\test-kalam")).display().to_string();
        assert_eq!(normalized, r"C:\git\test-kalam");
    }

    #[test]
    fn shell_working_directory_converts_extended_unc_paths() {
        if !cfg!(windows) {
            return;
        }
        let normalized = shell_working_directory(Path::new(r"\\?\UNC\server\share\project"))
            .display()
            .to_string();
        assert_eq!(normalized, r"\\server\share\project");
    }

    #[test]
    fn resolve_program_on_path_prefers_cmd_shims_over_extensionless_files() {
        if !cfg!(windows) {
            return;
        }
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path();
        std::fs::write(bin_dir.join("npm"), "#!/usr/bin/env node\n").expect("write npm shim");
        std::fs::write(bin_dir.join("npm.cmd"), "@echo off\r\n").expect("write npm.cmd");

        let original_path = env::var_os("PATH");
        env::set_var("PATH", bin_dir);

        let resolved = resolve_program_on_path("npm").expect("npm should resolve");
        assert_eq!(resolved.file_name().and_then(|name| name.to_str()), Some("npm.cmd"));

        match original_path {
            Some(value) => env::set_var("PATH", value),
            None => env::remove_var("PATH"),
        }
    }

    #[test]
    fn program_needs_shell_launch_detects_cmd_and_bat_files() {
        if !cfg!(windows) {
            return;
        }
        assert!(program_needs_shell_launch(Path::new(r"C:\nodejs\npm.cmd")));
        assert!(program_needs_shell_launch(Path::new(r"C:\tools\setup.bat")));
        assert!(!program_needs_shell_launch(Path::new(r"C:\nodejs\node.exe")));
    }
}
