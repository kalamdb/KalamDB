//! JavaScript package manager detection, selection, and install helpers.
//!
//! During `kalam init`, installed managers on `PATH` are discovered (npm, pnpm,
//! yarn, bun). Interactive setup prompts when more than one is available; the
//! choice is stored in `kalam.toml` as `[project].package_manager`.

use std::{env, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    error::{CLIError, Result},
    process::{resolve_program_on_path, run_path_tool},
    terminal_ui::SelectOption,
    workflow::project::{
        guidance::init_stage_context,
        prompts::prompt_select,
        ts::guidance::{
            init_missing_package_managers, init_package_install_failed,
            init_package_install_spawn_failed, init_package_manager_not_on_path,
        },
    },
};

/// Supported JavaScript package managers for TypeScript project scaffolding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub const ALL: &'static [PackageManager] = &[
        PackageManager::Pnpm,
        PackageManager::Bun,
        PackageManager::Yarn,
        PackageManager::Npm,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }

    pub fn program(self) -> &'static str {
        self.as_str()
    }

    pub fn install_args(self) -> &'static [&'static str] {
        match self {
            Self::Npm | Self::Pnpm | Self::Yarn | Self::Bun => &["install"],
        }
    }

    pub fn install_description(self) -> &'static str {
        match self {
            Self::Npm => "installing npm dependencies",
            Self::Pnpm => "installing pnpm dependencies",
            Self::Yarn => "installing yarn dependencies",
            Self::Bun => "installing bun dependencies",
        }
    }

    pub fn installed_success_message(self) -> &'static str {
        match self {
            Self::Npm => "installed npm dependencies",
            Self::Pnpm => "installed pnpm dependencies",
            Self::Yarn => "installed yarn dependencies",
            Self::Bun => "installed bun dependencies",
        }
    }

    /// Dev script command for `[dev.processes]` in `kalam.toml` (matches `package.json` `"dev"`).
    pub fn dev_run_command(self) -> &'static str {
        match self {
            Self::Npm => "npm run dev",
            Self::Pnpm => "pnpm dev",
            Self::Yarn => "yarn dev",
            Self::Bun => "bun run dev",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "npm" => Ok(Self::Npm),
            "pnpm" => Ok(Self::Pnpm),
            "yarn" => Ok(Self::Yarn),
            "bun" => Ok(Self::Bun),
            other => Err(CLIError::ConfigurationError(format!(
                "unsupported package manager '{other}'; supported: npm, pnpm, yarn, bun"
            ))),
        }
    }

    pub fn is_available(self) -> bool {
        resolve_program_on_path(self.program()).is_some()
    }
}

/// Package managers currently available on `PATH`.
pub fn detect_installed_package_managers() -> Vec<PackageManager> {
    PackageManager::ALL
        .iter()
        .copied()
        .filter(|manager| manager.is_available())
        .collect()
}

/// Infer the package manager used to invoke this CLI (npm/pnpm/yarn/bun), when set.
pub fn detect_invoking_package_manager() -> Option<PackageManager> {
    let user_agent = env::var("npm_config_user_agent").ok()?;
    parse_user_agent_package_manager(&user_agent)
}

fn parse_user_agent_package_manager(user_agent: &str) -> Option<PackageManager> {
    if user_agent.starts_with("pnpm/") {
        Some(PackageManager::Pnpm)
    } else if user_agent.starts_with("yarn/") {
        Some(PackageManager::Yarn)
    } else if user_agent.starts_with("bun/") {
        Some(PackageManager::Bun)
    } else if user_agent.starts_with("npm/") {
        Some(PackageManager::Npm)
    } else {
        None
    }
}

/// Detect the package manager for an existing project from lockfiles or `package.json`.
pub fn detect_package_manager_from_project(root: &Path) -> Option<PackageManager> {
    if root.join("bun.lock").exists() || root.join("bun.lockb").exists() {
        return Some(PackageManager::Bun);
    }
    if root.join("pnpm-lock.yaml").exists() {
        return Some(PackageManager::Pnpm);
    }
    if root.join("yarn.lock").exists() {
        return Some(PackageManager::Yarn);
    }
    if root.join("package-lock.json").exists() || root.join("npm-shrinkwrap.json").exists() {
        return Some(PackageManager::Npm);
    }
    package_manager_from_package_json(root)
}

fn package_manager_from_package_json(root: &Path) -> Option<PackageManager> {
    let package_json_path = root.join("package.json");
    let contents = std::fs::read_to_string(package_json_path).ok()?;
    let package_json: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let manager = package_json.get("packageManager")?.as_str()?.trim();

    if manager.starts_with("bun@") || manager == "bun" {
        Some(PackageManager::Bun)
    } else if manager.starts_with("pnpm@") || manager == "pnpm" {
        Some(PackageManager::Pnpm)
    } else if manager.starts_with("yarn@") || manager == "yarn" {
        Some(PackageManager::Yarn)
    } else if manager.starts_with("npm@") || manager == "npm" {
        Some(PackageManager::Npm)
    } else {
        None
    }
}

/// Pick a default from installed managers, preferring the invoking tool when available.
pub fn default_package_manager(installed: &[PackageManager]) -> Option<PackageManager> {
    if let Some(invoking) = detect_invoking_package_manager() {
        if installed.contains(&invoking) {
            return Some(invoking);
        }
    }

    for candidate in PackageManager::ALL {
        if installed.contains(candidate) {
            return Some(*candidate);
        }
    }

    None
}

pub struct PackageManagerInitOptions<'a> {
    pub explicit:        Option<PackageManager>,
    pub non_interactive: bool,
    pub color:           bool,
    pub detail:          &'a dyn Fn(&str),
}

/// Resolve which package manager to use during `kalam init` for TypeScript projects.
pub fn resolve_package_manager_for_init(
    options: PackageManagerInitOptions<'_>,
) -> Result<PackageManager> {
    if let Some(manager) = options.explicit {
        if !manager.is_available() {
            return Err(package_manager_missing_error(Some(manager)));
        }
        (options.detail)(&format!("Package manager: {}", manager.as_str()));
        return Ok(manager);
    }

    let installed = detect_installed_package_managers();
    if installed.is_empty() {
        return Err(package_manager_missing_error(None));
    }

    if installed.len() == 1 {
        let manager = installed[0];
        (options.detail)(&format!("Package manager: {}", manager.as_str()));
        return Ok(manager);
    }

    let default_manager = default_package_manager(&installed).expect("installed list is non-empty");

    if options.non_interactive {
        (options.detail)(&format!("Package manager: {}", default_manager.as_str()));
        return Ok(default_manager);
    }

    let select_options: Vec<SelectOption<'_>> =
        installed.iter().map(|manager| SelectOption::new(manager.as_str())).collect();
    let default_index =
        installed.iter().position(|manager| *manager == default_manager).unwrap_or(0);
    let selected =
        prompt_select("Package manager:", &select_options, default_index, options.color)?;
    let manager = installed[selected];
    (options.detail)(&format!("Package manager: {}", manager.as_str()));
    Ok(manager)
}

pub fn execute_package_install(root: &Path, manager: PackageManager) -> Result<()> {
    let args: Vec<&str> = manager.install_args().iter().copied().collect();
    let output = run_path_tool(manager.as_str(), &args, root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            package_manager_missing_error(Some(manager))
        } else {
            CLIError::ConfigurationError(init_package_install_spawn_failed(
                manager,
                root,
                &error.to_string(),
            ))
        }
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let exit_status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    Err(CLIError::ConfigurationError(init_package_install_failed(
        manager,
        root,
        &exit_status,
        stdout.trim(),
        stderr.trim(),
    )))
}

fn package_manager_missing_error(requested: Option<PackageManager>) -> CLIError {
    let installed = detect_installed_package_managers();
    let message = match requested {
        None => init_missing_package_managers(&installed),
        Some(manager) => init_package_manager_not_on_path(manager, &installed),
    };
    CLIError::ConfigurationError(init_stage_context(
        "selecting a JavaScript package manager",
        message,
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
    };

    use tempfile::TempDir;

    use super::*;

    struct EnvGuard {
        _env_lock:  std::sync::MutexGuard<'static, ()>,
        path:       Option<OsString>,
        user_agent: Option<OsString>,
    }

    impl EnvGuard {
        fn set_path(path: &Path) -> Self {
            let env_lock = crate::workflow::test_support::test_env_lock();
            let guard = Self {
                _env_lock:  env_lock,
                path:       env::var_os("PATH"),
                user_agent: env::var_os("npm_config_user_agent"),
            };
            env::set_var("PATH", path);
            env::remove_var("npm_config_user_agent");
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.path.take() {
                Some(value) => env::set_var("PATH", value),
                None => env::remove_var("PATH"),
            }
            match self.user_agent.take() {
                Some(value) => env::set_var("npm_config_user_agent", value),
                None => env::remove_var("npm_config_user_agent"),
            }
        }
    }

    fn write_fake_package_manager_executable(bin_dir: &Path, manager: PackageManager) {
        let name = manager.as_str();
        #[cfg(windows)]
        {
            fs::write(bin_dir.join(format!("{name}.cmd")), "@echo off\r\nexit /b 0\r\n").unwrap();
            // Node.js also ships an extensionless shim; keep it present so resolution
            // and install behavior match real Windows setups.
            fs::write(bin_dir.join(name), "#!/usr/bin/env node\nexit 0\n").unwrap();
        }
        #[cfg(not(windows))]
        {
            let path = bin_dir.join(name);
            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn isolated_bin_dir(managers: &[PackageManager]) -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        for manager in managers {
            write_fake_package_manager_executable(&bin_dir, *manager);
        }
        (temp, bin_dir)
    }

    #[test]
    fn package_manager_parse_accepts_known_names_and_rejects_unknown() {
        assert_eq!(PackageManager::parse("npm").unwrap(), PackageManager::Npm);
        assert_eq!(PackageManager::parse("PNPM").unwrap(), PackageManager::Pnpm);
        assert_eq!(PackageManager::parse(" yarn ").unwrap(), PackageManager::Yarn);
        assert_eq!(PackageManager::parse("bun").unwrap(), PackageManager::Bun);

        let error = PackageManager::parse("deno").unwrap_err().to_string();
        assert!(error.contains("unsupported package manager 'deno'"));
        assert!(error.contains("npm, pnpm, yarn, bun"));
    }

    #[test]
    fn package_manager_install_metadata_matches_manager() {
        assert_eq!(PackageManager::Pnpm.install_args(), &["install"]);
        assert_eq!(PackageManager::Bun.install_description(), "installing bun dependencies");
        assert_eq!(PackageManager::Yarn.installed_success_message(), "installed yarn dependencies");
    }

    #[test]
    fn package_manager_dev_run_command_matches_package_json_dev_script() {
        assert_eq!(PackageManager::Npm.dev_run_command(), "npm run dev");
        assert_eq!(PackageManager::Pnpm.dev_run_command(), "pnpm dev");
        assert_eq!(PackageManager::Yarn.dev_run_command(), "yarn dev");
        assert_eq!(PackageManager::Bun.dev_run_command(), "bun run dev");
    }

    #[test]
    fn parse_user_agent_detects_common_managers() {
        assert_eq!(
            parse_user_agent_package_manager("pnpm/9.0.0 npm/? node/v20.0.0"),
            Some(PackageManager::Pnpm)
        );
        assert_eq!(parse_user_agent_package_manager("yarn/4.0.0"), Some(PackageManager::Yarn));
        assert_eq!(parse_user_agent_package_manager("bun/1.2.0"), Some(PackageManager::Bun));
        assert_eq!(
            parse_user_agent_package_manager("npm/10.0.0 node/v20.0.0"),
            Some(PackageManager::Npm)
        );
    }

    #[test]
    fn default_package_manager_prefers_invoking_tool_when_installed() {
        let original = env::var_os("npm_config_user_agent");
        env::set_var("npm_config_user_agent", "pnpm/9.0.0");

        let default = default_package_manager(&[
            PackageManager::Npm,
            PackageManager::Pnpm,
            PackageManager::Bun,
        ]);
        assert_eq!(default, Some(PackageManager::Pnpm));

        match original {
            Some(value) => env::set_var("npm_config_user_agent", value),
            None => env::remove_var("npm_config_user_agent"),
        }
    }

    #[test]
    fn default_package_manager_uses_preference_order_without_invoking_hint() {
        let original = env::var_os("npm_config_user_agent");
        env::remove_var("npm_config_user_agent");

        let default = default_package_manager(&[
            PackageManager::Npm,
            PackageManager::Yarn,
            PackageManager::Bun,
        ]);
        assert_eq!(default, Some(PackageManager::Bun));

        match original {
            Some(value) => env::set_var("npm_config_user_agent", value),
            None => env::remove_var("npm_config_user_agent"),
        }
    }

    #[test]
    fn detect_package_manager_from_project_supports_known_lockfiles() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("bun.lock"), "").unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Bun));
        fs::remove_file(root.join("bun.lock")).unwrap();

        fs::write(root.join("bun.lockb"), "").unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Bun));
        fs::remove_file(root.join("bun.lockb")).unwrap();

        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Pnpm));
        fs::remove_file(root.join("pnpm-lock.yaml")).unwrap();

        fs::write(root.join("yarn.lock"), "").unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Yarn));
        fs::remove_file(root.join("yarn.lock")).unwrap();

        fs::write(root.join("package-lock.json"), "{}").unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Npm));
        fs::remove_file(root.join("package-lock.json")).unwrap();

        fs::write(root.join("npm-shrinkwrap.json"), "{}").unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Npm));
    }

    #[test]
    fn detect_package_manager_from_project_reads_package_manager_field() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("package.json"), r#"{ "packageManager": "bun@1.2.0" }"#).unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Bun));

        fs::write(root.join("package.json"), r#"{ "packageManager": "pnpm@10.0.0" }"#).unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Pnpm));

        fs::write(root.join("package.json"), r#"{ "packageManager": "yarn@4.0.0" }"#).unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Yarn));

        fs::write(root.join("package.json"), r#"{ "packageManager": "npm@10.0.0" }"#).unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Npm));
    }

    #[test]
    fn resolve_package_manager_for_init_honors_explicit_flag() {
        let (_temp, bin_dir) = isolated_bin_dir(&[PackageManager::Pnpm]);
        let _guard = EnvGuard::set_path(&bin_dir);

        let selected = resolve_package_manager_for_init(PackageManagerInitOptions {
            explicit:        Some(PackageManager::Pnpm),
            non_interactive: true,
            color:           false,
            detail:          &|_| {},
        })
        .expect("explicit manager should resolve");
        assert_eq!(selected, PackageManager::Pnpm);
    }

    #[test]
    fn detect_installed_package_managers_uses_isolated_path() {
        let (_temp, bin_dir) = isolated_bin_dir(&[PackageManager::Pnpm, PackageManager::Npm]);
        let _guard = EnvGuard::set_path(&bin_dir);

        assert_eq!(
            detect_installed_package_managers(),
            vec![PackageManager::Pnpm, PackageManager::Npm]
        );
    }

    #[test]
    fn resolve_package_manager_for_init_rejects_explicit_manager_not_on_path() {
        let (_temp, bin_dir) = isolated_bin_dir(&[PackageManager::Pnpm]);
        let _guard = EnvGuard::set_path(&bin_dir);

        let error = resolve_package_manager_for_init(PackageManagerInitOptions {
            explicit:        Some(PackageManager::Npm),
            non_interactive: true,
            color:           false,
            detail:          &|_| {},
        })
        .unwrap_err()
        .to_string();

        assert!(error.contains("npm was not found on PATH"));
        assert!(error.contains("--package-manager"));
    }

    #[test]
    fn resolve_package_manager_for_init_errors_when_none_installed() {
        let temp = TempDir::new().unwrap();
        let empty_bin = temp.path().join("empty-bin");
        fs::create_dir_all(&empty_bin).unwrap();
        let _guard = EnvGuard::set_path(&empty_bin);

        let error = resolve_package_manager_for_init(PackageManagerInitOptions {
            explicit:        None,
            non_interactive: true,
            color:           false,
            detail:          &|_| {},
        })
        .unwrap_err()
        .to_string();

        assert!(error.contains("no JavaScript package manager was found on PATH"));
        assert!(error.contains("How to fix:"));
    }

    #[test]
    fn resolve_package_manager_for_init_auto_selects_single_installed_manager() {
        let (_temp, bin_dir) = isolated_bin_dir(&[PackageManager::Yarn]);
        let _guard = EnvGuard::set_path(&bin_dir);

        let selected = resolve_package_manager_for_init(PackageManagerInitOptions {
            explicit:        None,
            non_interactive: false,
            color:           false,
            detail:          &|_| {},
        })
        .expect("single installed manager should resolve");

        assert_eq!(selected, PackageManager::Yarn);
    }

    #[test]
    fn resolve_package_manager_for_init_non_interactive_uses_preference_order() {
        let (_temp, bin_dir) = isolated_bin_dir(&[
            PackageManager::Npm,
            PackageManager::Yarn,
            PackageManager::Bun,
        ]);
        let _guard = EnvGuard::set_path(&bin_dir);

        let selected = resolve_package_manager_for_init(PackageManagerInitOptions {
            explicit:        None,
            non_interactive: true,
            color:           false,
            detail:          &|_| {},
        })
        .expect("non-interactive init should pick a default");

        assert_eq!(selected, PackageManager::Bun);
    }

    #[test]
    fn resolve_package_manager_for_init_non_interactive_prefers_invoking_manager() {
        let (_temp, bin_dir) = isolated_bin_dir(&[PackageManager::Npm, PackageManager::Pnpm]);
        let _guard = EnvGuard::set_path(&bin_dir);
        env::set_var("npm_config_user_agent", "pnpm/9.0.0 npm/? node/v20.0.0");

        let selected = resolve_package_manager_for_init(PackageManagerInitOptions {
            explicit:        None,
            non_interactive: true,
            color:           false,
            detail:          &|_| {},
        })
        .expect("invoking package manager should win");

        assert_eq!(selected, PackageManager::Pnpm);
    }

    #[test]
    fn detect_package_manager_from_project_prefers_lockfiles_in_priority_order() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("package-lock.json"), "{}").unwrap();
        fs::write(root.join("yarn.lock"), "").unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        fs::write(root.join("bun.lock"), "").unwrap();
        assert_eq!(detect_package_manager_from_project(root), Some(PackageManager::Bun));
    }

    #[test]
    fn detect_package_manager_from_project_returns_none_for_empty_project() {
        let temp = TempDir::new().unwrap();
        assert_eq!(detect_package_manager_from_project(temp.path()), None);
    }

    #[test]
    fn detect_invoking_package_manager_reads_npm_config_user_agent() {
        let original = env::var_os("npm_config_user_agent");
        env::set_var("npm_config_user_agent", "yarn/4.0.0");

        assert_eq!(detect_invoking_package_manager(), Some(PackageManager::Yarn));

        match original {
            Some(value) => env::set_var("npm_config_user_agent", value),
            None => env::remove_var("npm_config_user_agent"),
        }
    }

    #[test]
    fn default_package_manager_returns_none_for_empty_installed_list() {
        assert_eq!(default_package_manager(&[]), None);
    }

    #[test]
    fn execute_package_install_uses_manager_from_isolated_path() {
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let (_bin_temp, bin_dir) = isolated_bin_dir(&[PackageManager::Pnpm]);
        let _guard = EnvGuard::set_path(&bin_dir);

        execute_package_install(&project, PackageManager::Pnpm)
            .expect("fake pnpm install should succeed");
    }
}
