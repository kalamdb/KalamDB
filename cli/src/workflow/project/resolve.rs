//! Environment helpers for workflow commands.

use std::{collections::HashMap, env, fs, path::Path};

use kalamdb_commons::NamespaceId;

use crate::error::{CLIError, Result};

pub const ENV_VAR_KALAM_ENV: &str = "KALAM_ENV";
pub const ENV_VAR_KALAM_URL: &str = "KALAM_URL";
pub const ENV_VAR_KALAM_NAMESPACE: &str = "KALAM_NAMESPACE";
pub const ENV_VAR_KALAM_PROFILE: &str = "KALAM_PROFILE";
const PROJECT_ENV_FILE: &str = ".env";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    CliFlag,
    EnvironmentVariable,
    ProjectConfig,
    InstanceState,
    DefaultDev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEnvironment {
    pub name:             String,
    pub url:              String,
    pub namespace:        NamespaceId,
    pub env_source:       ResolutionSource,
    pub url_source:       ResolutionSource,
    pub namespace_source: ResolutionSource,
}

/// Map a workflow environment name to a credential instance key.
pub fn credential_instance_for_env(env_name: &str) -> String {
    format!("kalam-{env_name}")
}

/// Resolve the saved CLI credential profile selected for this project.
///
/// `KALAM_PROFILE` from the process environment wins over the project `.env` file.
pub fn resolve_kalam_profile(project_root: &Path) -> Result<Option<String>> {
    if let Ok(profile) = env::var(ENV_VAR_KALAM_PROFILE) {
        let trimmed = profile.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
    }

    read_project_env_value(project_root, ENV_VAR_KALAM_PROFILE)
}

/// Load key/value pairs from the project `.env` file.
///
/// Existing process environment variables are not applied here; callers decide
/// whether to inject values into child processes. Empty values are omitted.
/// The first assignment for a key wins.
pub fn load_project_dotenv(project_root: &Path) -> Result<HashMap<String, String>> {
    let env_path = project_root.join(PROJECT_ENV_FILE);
    if !env_path.is_file() {
        return Ok(HashMap::new());
    }

    let contents = fs::read_to_string(&env_path).map_err(|error| {
        CLIError::FileError(format!(
            "failed to read project environment file '{}': {error}",
            env_path.display()
        ))
    })?;
    Ok(parse_dotenv(&contents))
}

fn read_project_env_value(project_root: &Path, key: &str) -> Result<Option<String>> {
    Ok(load_project_dotenv(project_root)?.get(key).cloned())
}

fn parse_dotenv(contents: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((candidate_key, candidate_value)) = line.split_once('=') else {
            continue;
        };
        let key = candidate_key.trim();
        if key.is_empty() || vars.contains_key(key) {
            continue;
        }

        let value = candidate_value.trim().trim_matches('"').trim_matches('\'').trim();
        if value.is_empty() {
            continue;
        }
        vars.insert(key.to_string(), value.to_string());
    }
    vars
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn resolve_kalam_profile_reads_project_env_file() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(".env"), "KALAM_PROFILE=prod-admin\n").unwrap();

        let profile = resolve_kalam_profile(temp.path()).unwrap();

        assert_eq!(profile.as_deref(), Some("prod-admin"));
    }

    #[test]
    fn resolve_kalam_profile_prefers_process_env() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(".env"), "KALAM_PROFILE=prod-admin\n").unwrap();
        std::env::set_var(ENV_VAR_KALAM_PROFILE, "shell-profile");

        let profile = resolve_kalam_profile(temp.path()).unwrap();

        assert_eq!(profile.as_deref(), Some("shell-profile"));
        std::env::remove_var(ENV_VAR_KALAM_PROFILE);
    }

    #[test]
    fn load_project_dotenv_parses_password_and_skips_empty_values() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(".env"),
            concat!(
                "# comment\n",
                "export KALAM_PASSWORD=kalamdb123\n",
                "KALAM_PASSWORD=ignored-second\n",
                "KALAM_USER=root\n",
                "EMPTY=\n",
                "QUOTED=\"quoted-value\"\n",
            ),
        )
        .unwrap();

        let vars = load_project_dotenv(temp.path()).unwrap();

        assert_eq!(vars.get("KALAM_PASSWORD").map(String::as_str), Some("kalamdb123"));
        assert_eq!(vars.get("KALAM_USER").map(String::as_str), Some("root"));
        assert_eq!(vars.get("QUOTED").map(String::as_str), Some("quoted-value"));
        assert!(!vars.contains_key("EMPTY"));
    }

    #[test]
    fn load_project_dotenv_returns_empty_when_file_missing() {
        let temp = TempDir::new().unwrap();
        let vars = load_project_dotenv(temp.path()).unwrap();
        assert!(vars.is_empty());
    }
}
