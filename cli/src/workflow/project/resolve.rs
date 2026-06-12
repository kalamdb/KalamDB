//! Environment resolution for workflow commands.

use std::{env, fs, path::Path};

use kalam_client::CredentialStore;
use kalamdb_commons::NamespaceId;

use crate::{
    credentials::FileCredentialStore,
    error::{CLIError, Result},
    workflow::project::{
        config::{ConnectionEnv, KalamProjectConfig},
        identifiers::parse_namespace_id,
    },
};

pub const ENV_VAR_KALAM_ENV: &str = "KALAM_ENV";
pub const ENV_VAR_KALAM_URL: &str = "KALAM_URL";
pub const ENV_VAR_KALAM_NAMESPACE: &str = "KALAM_NAMESPACE";
pub const ENV_VAR_KALAM_PROFILE: &str = "KALAM_PROFILE";
const PROJECT_ENV_FILE: &str = ".env";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionSource {
    CliFlag,
    EnvironmentVariable,
    ProjectConfig,
    DefaultDev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEnvironment {
    pub name: String,
    pub url: String,
    pub namespace: NamespaceId,
    pub env_source: ResolutionSource,
    pub url_source: ResolutionSource,
    pub namespace_source: ResolutionSource,
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentOverrides<'a> {
    pub env: Option<&'a str>,
    pub url: Option<&'a str>,
    pub namespace: Option<&'a str>,
}

pub fn resolve_environment(
    config: &KalamProjectConfig,
    overrides: &EnvironmentOverrides<'_>,
) -> Result<ResolvedEnvironment> {
    let (name, env_source) = resolve_env_name(config, overrides.env)?;
    let connection = config.connection.get(&name).ok_or_else(|| {
        CLIError::ConfigurationError(format!("no [connection.{name}] section in kalam.toml"))
    })?;

    let (url, url_source) = resolve_url(connection, overrides.url)?;
    let (namespace, namespace_source) = resolve_namespace(connection, overrides.namespace)?;

    Ok(ResolvedEnvironment {
        name,
        url,
        namespace,
        env_source,
        url_source,
        namespace_source,
    })
}

fn resolve_env_name(
    config: &KalamProjectConfig,
    cli_env: Option<&str>,
) -> Result<(String, ResolutionSource)> {
    if let Some(env) = cli_env.map(str::trim).filter(|v| !v.is_empty()) {
        return Ok((env.to_string(), ResolutionSource::CliFlag));
    }
    if let Ok(env) = env::var(ENV_VAR_KALAM_ENV) {
        let trimmed = env.trim();
        if !trimmed.is_empty() {
            return Ok((trimmed.to_string(), ResolutionSource::EnvironmentVariable));
        }
    }
    if !config.project.default_env.trim().is_empty() {
        return Ok((config.project.default_env.clone(), ResolutionSource::ProjectConfig));
    }
    Ok(("dev".to_string(), ResolutionSource::DefaultDev))
}

fn resolve_url(
    connection: &ConnectionEnv,
    cli_url: Option<&str>,
) -> Result<(String, ResolutionSource)> {
    if let Some(url) = cli_url.map(str::trim).filter(|v| !v.is_empty()) {
        return Ok((url.to_string(), ResolutionSource::CliFlag));
    }
    if let Ok(url) = env::var(ENV_VAR_KALAM_URL) {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Ok((trimmed.to_string(), ResolutionSource::EnvironmentVariable));
        }
    }
    Ok((connection.url.clone(), ResolutionSource::ProjectConfig))
}

fn resolve_namespace(
    connection: &ConnectionEnv,
    cli_namespace: Option<&str>,
) -> Result<(NamespaceId, ResolutionSource)> {
    if let Some(namespace) = cli_namespace.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok((parse_namespace_id(namespace)?, ResolutionSource::CliFlag));
    }

    if let Ok(namespace) = env::var(ENV_VAR_KALAM_NAMESPACE) {
        let trimmed = namespace.trim();
        if !trimmed.is_empty() {
            return Ok((parse_namespace_id(trimmed)?, ResolutionSource::EnvironmentVariable));
        }
    }

    Ok((connection.namespace.clone(), ResolutionSource::ProjectConfig))
}

/// Map a workflow environment name to a credential instance key.
pub fn credential_instance_for_env(env_name: &str) -> String {
    format!("kalam-{env_name}")
}

/// Resolve stored credentials for a workflow environment without embedding secrets in kalam.toml.
pub fn load_env_credentials(
    store: &FileCredentialStore,
    env_name: &str,
) -> Result<Option<kalam_client::credentials::Credentials>> {
    let instance = credential_instance_for_env(env_name);
    store
        .get_credentials(&instance)
        .map_err(|e| CLIError::ConfigurationError(format!("failed to load credentials: {e}")))
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

fn read_project_env_value(project_root: &Path, key: &str) -> Result<Option<String>> {
    let env_path = project_root.join(PROJECT_ENV_FILE);
    if !env_path.is_file() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&env_path).map_err(|error| {
        CLIError::FileError(format!(
            "failed to read project environment file '{}': {error}",
            env_path.display()
        ))
    })?;

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((candidate_key, candidate_value)) = line.split_once('=') else {
            continue;
        };
        if candidate_key.trim() != key {
            continue;
        }

        let value = candidate_value.trim().trim_matches('"').trim_matches('\'').trim();
        if value.is_empty() {
            return Ok(None);
        }
        return Ok(Some(value.to_string()));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use super::*;
    use kalamdb_commons::NamespaceId;

    use crate::workflow::project::config::{
        ConnectionEnv, ProjectSection, SchemaMode, SchemaSection, SchemaTarget,
    };
    use tempfile::TempDir;

    fn sample_config() -> KalamProjectConfig {
        KalamProjectConfig {
            project: ProjectSection {
                name: "demo".into(),
                default_env: "dev".into(),
                package_manager: None,
                kalam_dir: "kalam".into(),
            },
            connection: HashMap::from([
                (
                    "dev".into(),
                    ConnectionEnv {
                        url: "http://localhost:2900".into(),
                        namespace: NamespaceId::new("app"),
                    },
                ),
                (
                    "prod".into(),
                    ConnectionEnv {
                        url: "https://db.example.com".into(),
                        namespace: NamespaceId::new("app"),
                    },
                ),
            ]),
            schema: SchemaSection {
                mode: SchemaMode::Sql,
                path: Some("schema.sql".into()),
                watch: true,
                languages: vec!["typescript".into()],
                targets: HashMap::from([(
                    "typescript".into(),
                    SchemaTarget {
                        output: "src/generated/kalam.ts".into(),
                    },
                )]),
            },
            migrations: Default::default(),
            dev: Default::default(),
            logging: Default::default(),
        }
    }

    #[test]
    fn cli_flag_overrides_config() {
        let config = sample_config();
        let resolved = resolve_environment(
            &config,
            &EnvironmentOverrides {
                env: Some("prod"),
                url: Some("https://override.example.com"),
                namespace: Some("other"),
            },
        )
        .unwrap();

        assert_eq!(resolved.name, "prod");
        assert_eq!(resolved.url, "https://override.example.com");
        assert_eq!(resolved.namespace, NamespaceId::new("other"));
        assert_eq!(resolved.env_source, ResolutionSource::CliFlag);
    }

    #[test]
    fn default_env_from_config() {
        let config = sample_config();
        let resolved = resolve_environment(&config, &EnvironmentOverrides::default()).unwrap();
        assert_eq!(resolved.name, "dev");
        assert_eq!(resolved.url, "http://localhost:2900");
    }

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
}
