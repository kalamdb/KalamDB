//! Shared target resolution for SQL and project workflow commands.
//!
//! Every everyday command (`dev`, `up`, `down`, `status`, `logs`, SQL) should
//! call [`resolve_target`] instead of inventing its own URL lookup.
//!
//! Selection order:
//! 1. Explicit CLI flags (`--global`, `--env`, `--url`, `--host`)
//! 2. `KALAM_ENV` / `KALAM_URL` / `KALAM_NAMESPACE`
//! 3. `kalam.toml`
//! 4. Project-local runtime in the current directory, or `http://127.0.0.1:2900`

use std::path::{Path, PathBuf};

use kalamdb_commons::NamespaceId;

use crate::{
    error::{CLIError, Result},
    workflow::{
        instance::{self, default_http_url, ManagedLayout},
        project::{
            config::{ConnectionEnv, EnvironmentPurpose, KalamProjectConfig},
            identifiers::{normalize_namespace_name, parse_namespace_id},
            resolve::{
                credential_instance_for_env, ResolvedEnvironment, ENV_VAR_KALAM_ENV,
                ENV_VAR_KALAM_NAMESPACE, ENV_VAR_KALAM_URL,
            },
        },
    },
};

pub const DEFAULT_LOCAL_URL: &str = "http://127.0.0.1:2900";
pub const SHARED_CREDENTIAL_INSTANCE: &str = "kalam-global";
pub const LOCAL_CREDENTIAL_INSTANCE: &str = "kalam-local";

pub use instance::{
    default_http_url as http_url, TargetKind, DEFAULT_HTTP_PORT, SHARED_SERVER_NAME,
};

pub use crate::workflow::project::resolve::ResolutionSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSelector {
    pub start_dir:   PathBuf,
    pub project_dir: Option<PathBuf>,
    pub global:      bool,
    pub env:         Option<String>,
    pub url:         Option<String>,
    pub host:        Option<String>,
    pub port:        Option<u16>,
    pub namespace:   Option<String>,
    pub instance:    Option<String>,
}

impl TargetSelector {
    pub fn new(start_dir: impl Into<PathBuf>) -> Self {
        Self {
            start_dir:   start_dir.into(),
            project_dir: None,
            global:      false,
            env:         None,
            url:         None,
            host:        None,
            port:        None,
            namespace:   None,
            instance:    None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub kind:                TargetKind,
    pub environment_name:    String,
    pub purpose:             EnvironmentPurpose,
    pub url:                 String,
    pub namespace:           NamespaceId,
    pub credential_instance: String,
    pub project_root:        Option<PathBuf>,
    pub project_name:        Option<String>,
    pub layout:              Option<ManagedLayout>,
    pub env_source:          ResolutionSource,
    pub url_source:          ResolutionSource,
    pub namespace_source:    ResolutionSource,
}

impl ResolvedTarget {
    pub fn is_managed_local(&self) -> bool {
        matches!(self.kind, TargetKind::ProjectLocal | TargetKind::SharedLocal)
    }

    pub fn display_kind(&self) -> &'static str {
        match self.kind {
            TargetKind::ProjectLocal => "local",
            TargetKind::SharedLocal => "global",
            TargetKind::Remote => "remote",
        }
    }

    pub fn to_environment(&self) -> ResolvedEnvironment {
        ResolvedEnvironment {
            name:             self.environment_name.clone(),
            url:              self.url.clone(),
            namespace:        self.namespace.clone(),
            env_source:       self.env_source,
            url_source:       self.url_source,
            namespace_source: self.namespace_source,
        }
    }
}

/// Resolve the target shared by lifecycle, workflow, and SQL commands.
pub fn resolve_target(selector: &TargetSelector) -> Result<ResolvedTarget> {
    resolve_target_inner(selector, None)
}

/// Resolve using an already-loaded `kalam.toml` (tests and workflow commands).
pub fn resolve_loaded_target(
    selector: &TargetSelector,
    project_root: PathBuf,
    config: &KalamProjectConfig,
) -> Result<ResolvedTarget> {
    resolve_target_inner(selector, Some((project_root, config)))
}

fn resolve_target_inner(
    selector: &TargetSelector,
    loaded: Option<(PathBuf, &KalamProjectConfig)>,
) -> Result<ResolvedTarget> {
    validate_selector(selector)?;

    if selector.global {
        return resolve_shared_local(selector);
    }

    if let Some(url) = selector.url.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        return resolve_explicit_url(selector, url, ResolutionSource::CliFlag, loaded);
    }

    if let Some(host) = selector.host.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        let port = selector.port.unwrap_or(DEFAULT_HTTP_PORT);
        let url = format!("http://{host}:{port}");
        return resolve_explicit_url(selector, &url, ResolutionSource::CliFlag, loaded);
    }

    if let Ok(url) = std::env::var(ENV_VAR_KALAM_URL) {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return resolve_explicit_url(
                selector,
                trimmed,
                ResolutionSource::EnvironmentVariable,
                loaded,
            );
        }
    }

    if let Some((project_root, config)) = loaded {
        return resolve_project_target(selector, project_root, config.clone());
    }

    match discover_project(selector) {
        Ok((project_root, config)) => resolve_project_target(selector, project_root, config),
        Err(error) if selector.env.as_deref().is_some_and(|value| !value.trim().is_empty()) => {
            Err(error)
        },
        Err(_) => resolve_directory_local(selector),
    }
}

/// Resolve the database that `up` / `down` / `logs` should manage.
///
/// These commands always operate on a local process. A remote `default_env`
/// must not steal `kalam up` away from this project's `.kalam/` (or
/// `kalam/server`) runtime. Explicit `--env` / `--url` / `--host` that resolve
/// to a remote target are rejected.
pub fn resolve_lifecycle_target(selector: &TargetSelector) -> Result<ResolvedTarget> {
    validate_selector(selector)?;
    if selector.global {
        return resolve_shared_local(selector);
    }
    let explicit_remote_selector =
        selector.env.as_deref().is_some_and(|value| !value.trim().is_empty())
            || selector.url.as_deref().is_some_and(|value| !value.trim().is_empty())
            || selector.host.as_deref().is_some_and(|value| !value.trim().is_empty());
    if explicit_remote_selector {
        let target = resolve_target(selector)?;
        if !target.is_managed_local() {
            return Err(CLIError::ConfigurationError(
                "`kalam up`, `kalam down`, and `kalam logs` manage local databases only; use \
                 `--global` or a project-local directory, not a remote `--env`"
                    .into(),
            ));
        }
        return Ok(target);
    }
    match discover_project(selector) {
        Ok((project_root, config)) => resolve_project_local_runtime(selector, project_root, config),
        Err(_) => resolve_directory_local(selector),
    }
}

fn resolve_project_local_runtime(
    selector: &TargetSelector,
    project_root: PathBuf,
    config: KalamProjectConfig,
) -> Result<ResolvedTarget> {
    let mut local_selector = selector.clone();
    local_selector.start_dir = project_root.clone();
    let mut target = resolve_directory_local(&local_selector)?;
    target.project_root = Some(project_root.clone());
    target.project_name = Some(config.project.name.clone());
    let loopback = config.connection.values().find(|connection| {
        crate::workflow::project::connection_url::is_loopback_server_url(&connection.url)
    });
    if let Some(connection) = loopback {
        let namespace = override_namespace(selector, Some(&connection.namespace))?;
        target.namespace = namespace.0;
        target.namespace_source = namespace.1;
        target.purpose = resolved_purpose(&config.project.default_env, Some(connection));
    } else {
        target.namespace = parse_namespace_id(&normalize_namespace_name(&config.project.name))
            .or_else(|_| parse_namespace_id("app"))?;
        target.namespace_source = ResolutionSource::ProjectConfig;
    }
    Ok(target)
}

fn validate_selector(selector: &TargetSelector) -> Result<()> {
    if selector.global
        && (selector.env.as_deref().is_some_and(|value| !value.trim().is_empty())
            || selector.url.as_deref().is_some_and(|value| !value.trim().is_empty())
            || selector.host.as_deref().is_some_and(|value| !value.trim().is_empty()))
    {
        return Err(CLIError::ConfigurationError(
            "`--global` cannot be combined with `--env`, `--url`, or `--host`".into(),
        ));
    }
    Ok(())
}

fn discover_project(selector: &TargetSelector) -> Result<(PathBuf, KalamProjectConfig)> {
    KalamProjectConfig::discover(&selector.start_dir, selector.project_dir.as_deref())
}

fn resolve_shared_local(selector: &TargetSelector) -> Result<ResolvedTarget> {
    let layout = instance::shared_layout();
    let persisted = instance::load_instance(&layout).ok().flatten();
    let url = persisted
        .as_ref()
        .map(|record| record.url.clone())
        .unwrap_or_else(|| default_http_url(DEFAULT_HTTP_PORT));
    let (namespace, namespace_source, project_root, project_name, credential_instance) =
        match discover_project(selector) {
            Ok((project_root, config)) => {
                let env_name = config.project.default_env.clone();
                let connection = config.connection.get(&env_name);
                let namespace =
                    override_namespace(selector, connection.map(|item| &item.namespace))?;
                let credential = credential_instance_for_selector(selector, &env_name);
                (
                    namespace.0,
                    namespace.1,
                    Some(project_root),
                    Some(config.project.name),
                    credential,
                )
            },
            Err(_) => (
                parse_namespace_id("app")?,
                ResolutionSource::DefaultDev,
                None,
                None,
                SHARED_CREDENTIAL_INSTANCE.to_string(),
            ),
        };

    Ok(ResolvedTarget {
        kind: TargetKind::SharedLocal,
        environment_name: SHARED_SERVER_NAME.to_string(),
        purpose: EnvironmentPurpose::Development,
        url,
        namespace,
        credential_instance,
        project_root,
        project_name,
        layout: Some(layout),
        env_source: ResolutionSource::CliFlag,
        url_source: if persisted.is_some() {
            ResolutionSource::InstanceState
        } else {
            ResolutionSource::DefaultDev
        },
        namespace_source,
    })
}

fn resolve_explicit_url(
    selector: &TargetSelector,
    url: &str,
    url_source: ResolutionSource,
    loaded: Option<(PathBuf, &KalamProjectConfig)>,
) -> Result<ResolvedTarget> {
    let (project_root, config) = match loaded {
        Some((root, config)) => (Some(root), Some(config.clone())),
        None => discover_project(selector).ok().unzip(),
    };
    let env_name = env_name_from(selector, config.as_ref())?;
    let connection = config.as_ref().and_then(|item| item.connection.get(&env_name.0));
    let namespace = match override_namespace(selector, connection.map(|item| &item.namespace)) {
        Ok(value) => value,
        Err(_) => (
            namespace_from_dir(project_root.as_deref().unwrap_or(&selector.start_dir))?,
            ResolutionSource::DefaultDev,
        ),
    };
    let purpose = resolved_purpose(&env_name.0, connection);
    let auto_start_db = config.as_ref().is_none_or(|item| item.dev.auto_start_db);
    let kind = managed_target_kind(url, auto_start_db);
    let layout = project_root
        .as_ref()
        .and_then(|root| (kind != TargetKind::Remote).then(|| instance::project_layout(root)));

    Ok(ResolvedTarget {
        kind,
        environment_name: env_name.0.clone(),
        purpose,
        url: url.trim_end_matches('/').to_string(),
        namespace: namespace.0,
        credential_instance: credential_instance_for_selector(selector, &env_name.0),
        project_root,
        project_name: config.as_ref().map(|item| item.project.name.clone()),
        layout,
        env_source: env_name.1,
        url_source,
        namespace_source: namespace.1,
    })
}

fn resolve_project_target(
    selector: &TargetSelector,
    project_root: PathBuf,
    config: KalamProjectConfig,
) -> Result<ResolvedTarget> {
    let env_name = env_name_from(selector, Some(&config))?;
    let connection = config.connection.get(&env_name.0).ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "no [connection.{}] section in kalam.toml",
            env_name.0
        ))
    })?;
    let layout = instance::project_layout(&project_root);
    let persisted = instance::load_instance(&layout).ok().flatten();
    let loopback =
        crate::workflow::project::connection_url::is_loopback_server_url(&connection.url);
    let (url, url_source) = if config.dev.auto_start_db && loopback {
        if let Some(record) = persisted.as_ref() {
            (record.url.clone(), ResolutionSource::InstanceState)
        } else {
            (connection.url.clone(), ResolutionSource::ProjectConfig)
        }
    } else {
        (connection.url.clone(), ResolutionSource::ProjectConfig)
    };
    let namespace = override_namespace(selector, Some(&connection.namespace))?;
    let kind = managed_target_kind(&url, config.dev.auto_start_db);

    Ok(ResolvedTarget {
        kind,
        environment_name: env_name.0.clone(),
        purpose: resolved_purpose(&env_name.0, Some(connection)),
        url,
        namespace: namespace.0,
        credential_instance: credential_instance_for_selector(selector, &env_name.0),
        project_root: Some(project_root),
        project_name: Some(config.project.name),
        layout: Some(layout).filter(|_| kind != TargetKind::Remote),
        env_source: env_name.1,
        url_source,
        namespace_source: namespace.1,
    })
}

fn resolve_directory_local(selector: &TargetSelector) -> Result<ResolvedTarget> {
    let root = selector.start_dir.clone();
    let layout = instance::project_layout(&root);
    let persisted = instance::load_instance(&layout).ok().flatten();
    let url = persisted
        .as_ref()
        .map(|record| record.url.clone())
        .unwrap_or_else(|| default_http_url(DEFAULT_HTTP_PORT));
    let namespace = match override_namespace(selector, None) {
        Ok(value) => value,
        Err(_) => (namespace_from_dir(&root)?, ResolutionSource::DefaultDev),
    };

    Ok(ResolvedTarget {
        kind: TargetKind::ProjectLocal,
        environment_name: "local".into(),
        purpose: EnvironmentPurpose::Development,
        url,
        namespace: namespace.0,
        credential_instance: selector
            .instance
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| LOCAL_CREDENTIAL_INSTANCE.to_string()),
        project_root: Some(root),
        project_name: None,
        layout: Some(layout),
        env_source: ResolutionSource::DefaultDev,
        url_source: if persisted.is_some() {
            ResolutionSource::InstanceState
        } else {
            ResolutionSource::DefaultDev
        },
        namespace_source: namespace.1,
    })
}

fn env_name_from(
    selector: &TargetSelector,
    config: Option<&KalamProjectConfig>,
) -> Result<(String, ResolutionSource)> {
    if let Some(env) = selector.env.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        return Ok((env.to_string(), ResolutionSource::CliFlag));
    }
    if let Ok(env) = std::env::var(ENV_VAR_KALAM_ENV) {
        let trimmed = env.trim();
        if !trimmed.is_empty() {
            return Ok((trimmed.to_string(), ResolutionSource::EnvironmentVariable));
        }
    }
    if let Some(config) = config {
        if !config.project.default_env.trim().is_empty() {
            return Ok((config.project.default_env.clone(), ResolutionSource::ProjectConfig));
        }
    }
    Ok(("dev".to_string(), ResolutionSource::DefaultDev))
}

fn override_namespace(
    selector: &TargetSelector,
    configured: Option<&NamespaceId>,
) -> Result<(NamespaceId, ResolutionSource)> {
    if let Some(namespace) =
        selector.namespace.as_deref().map(str::trim).filter(|value| !value.is_empty())
    {
        return Ok((parse_namespace_id(namespace)?, ResolutionSource::CliFlag));
    }
    if let Ok(namespace) = std::env::var(ENV_VAR_KALAM_NAMESPACE) {
        let trimmed = namespace.trim();
        if !trimmed.is_empty() {
            return Ok((parse_namespace_id(trimmed)?, ResolutionSource::EnvironmentVariable));
        }
    }
    if let Some(namespace) = configured {
        return Ok((namespace.clone(), ResolutionSource::ProjectConfig));
    }
    Err(CLIError::ConfigurationError("namespace is not configured".into()))
}

fn credential_instance_for_selector(selector: &TargetSelector, env_name: &str) -> String {
    selector
        .instance
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| credential_instance_for_env(env_name))
}

fn resolved_purpose(env_name: &str, connection: Option<&ConnectionEnv>) -> EnvironmentPurpose {
    if let Some(purpose) = connection.and_then(|item| item.purpose) {
        return purpose;
    }
    EnvironmentPurpose::from_env_name(env_name)
}

fn managed_target_kind(url: &str, auto_start_db: bool) -> TargetKind {
    if auto_start_db && crate::workflow::project::connection_url::is_loopback_server_url(url) {
        TargetKind::ProjectLocal
    } else {
        TargetKind::Remote
    }
}

fn namespace_from_dir(path: &Path) -> Result<NamespaceId> {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("default");
    let normalized = normalize_namespace_name(name);
    parse_namespace_id(&normalized).or_else(|_| parse_namespace_id("app"))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::workflow::test_support::{env_lock, multi_env_resolve_test_config};

    fn write_project(temp: &TempDir) -> PathBuf {
        let root = temp.path().join("app");
        std::fs::create_dir_all(&root).unwrap();
        multi_env_resolve_test_config().save_to_path(&root.join("kalam.toml")).unwrap();
        root
    }

    #[test]
    fn rejects_global_with_env() {
        let _lock = env_lock::lock();
        let selector = TargetSelector {
            global: true,
            env: Some("production".into()),
            ..TargetSelector::new(".")
        };
        let error = resolve_target(&selector).unwrap_err().to_string();
        assert!(error.contains("--global"));
        assert!(error.contains("--env"));
    }

    #[test]
    fn rejects_global_with_url() {
        let selector = TargetSelector {
            global: true,
            url: Some("https://db.example.com".into()),
            ..TargetSelector::new(".")
        };
        assert!(resolve_target(&selector).is_err());
    }

    #[test]
    fn project_env_flag_selects_named_connection() {
        let _lock = env_lock::lock();
        let _env = env_lock::unset(ENV_VAR_KALAM_ENV);
        let _url = env_lock::unset(ENV_VAR_KALAM_URL);
        let temp = TempDir::new().unwrap();
        let root = write_project(&temp);
        let selector = TargetSelector {
            start_dir: root.clone(),
            env: Some("prod".into()),
            ..TargetSelector::new(&root)
        };
        let target = resolve_target(&selector).unwrap();
        assert_eq!(target.environment_name, "prod");
        assert_eq!(target.url, "https://db.example.com");
        assert_eq!(target.kind, TargetKind::Remote);
        assert!(target.layout.is_none());
        assert_eq!(target.purpose, EnvironmentPurpose::Production);
    }

    #[test]
    fn project_default_env_selects_dev_connection() {
        let _lock = env_lock::lock();
        let _env = env_lock::unset(ENV_VAR_KALAM_ENV);
        let _url = env_lock::unset(ENV_VAR_KALAM_URL);
        let temp = TempDir::new().unwrap();
        let root = write_project(&temp);
        let target = resolve_target(&TargetSelector::new(&root)).unwrap();
        assert_eq!(target.environment_name, "dev");
        assert_eq!(target.url, "http://localhost:2900");
        assert_eq!(target.env_source, ResolutionSource::ProjectConfig);
        assert_eq!(target.kind, TargetKind::ProjectLocal);
    }

    #[test]
    fn loopback_url_is_remote_when_auto_start_db_is_disabled() {
        let _lock = env_lock::lock();
        let _env = env_lock::unset(ENV_VAR_KALAM_ENV);
        let _url = env_lock::unset(ENV_VAR_KALAM_URL);
        let temp = TempDir::new().unwrap();
        let root = write_project(&temp);
        let mut config = crate::workflow::project::config::KalamProjectConfig::load_from_path(
            &root.join("kalam.toml"),
        )
        .unwrap();
        config.dev.auto_start_db = false;
        config.save_to_path(&root.join("kalam.toml")).unwrap();

        let target = resolve_target(&TargetSelector::new(&root)).unwrap();
        assert_eq!(target.url, "http://localhost:2900");
        assert_eq!(target.kind, TargetKind::Remote);
        assert!(target.layout.is_none());
    }

    #[test]
    fn empty_directory_resolves_to_project_local_runtime() {
        let _lock = env_lock::lock();
        let _env = env_lock::unset(ENV_VAR_KALAM_ENV);
        let _url = env_lock::unset(ENV_VAR_KALAM_URL);
        let temp = TempDir::new().unwrap();
        let selector = TargetSelector::new(temp.path());
        let target = resolve_target(&selector).unwrap();
        assert_eq!(target.kind, TargetKind::ProjectLocal);
        assert_eq!(target.url, DEFAULT_LOCAL_URL);
        assert!(target.layout.is_some());
    }

    #[test]
    fn host_without_port_uses_2900() {
        let selector = TargetSelector {
            host: Some("127.0.0.1".into()),
            namespace: Some("app".into()),
            ..TargetSelector::new(".")
        };
        let target = resolve_target(&selector).unwrap();
        assert_eq!(target.url, "http://127.0.0.1:2900");
    }

    #[test]
    fn reserved_directory_name_falls_back_to_app_namespace() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("default");
        std::fs::create_dir_all(&root).unwrap();
        let namespace = namespace_from_dir(&root).unwrap();
        assert_eq!(namespace.as_str(), "app");
        assert_eq!(namespace_from_dir(Path::new(".")).unwrap().as_str(), "app");
    }

    #[test]
    fn loopback_instance_url_overrides_project_config() {
        let _lock = env_lock::lock();
        let _env = env_lock::unset(ENV_VAR_KALAM_ENV);
        let _url = env_lock::unset(ENV_VAR_KALAM_URL);
        let temp = TempDir::new().unwrap();
        let root = write_project(&temp);
        let layout = instance::project_layout(&root);
        layout.ensure_dirs().unwrap();
        let mut record = instance::new_instance_record(
            TargetKind::ProjectLocal,
            &layout,
            2911,
            None,
            Some("app"),
            instance::StartedBy::Up,
            true,
            None,
            Some(root.clone()),
        );
        record.url = "http://127.0.0.1:2911".into();
        instance::save_instance(&layout, &record).unwrap();

        let target = resolve_target(&TargetSelector::new(&root)).unwrap();
        assert_eq!(target.url, "http://127.0.0.1:2911");
        assert_eq!(target.url_source, ResolutionSource::InstanceState);
        assert_eq!(target.kind, TargetKind::ProjectLocal);
    }

    #[test]
    fn lifecycle_target_stays_local_when_default_env_is_remote() {
        let _lock = env_lock::lock();
        let _env = env_lock::unset(ENV_VAR_KALAM_ENV);
        let _url = env_lock::unset(ENV_VAR_KALAM_URL);
        let temp = TempDir::new().unwrap();
        let root = write_project(&temp);
        let mut config = crate::workflow::project::config::KalamProjectConfig::load_from_path(
            &root.join("kalam.toml"),
        )
        .unwrap();
        config.project.default_env = "prod".into();
        config.save_to_path(&root.join("kalam.toml")).unwrap();

        let resolved = resolve_target(&TargetSelector::new(&root)).unwrap();
        assert_eq!(resolved.kind, TargetKind::Remote);

        let lifecycle = resolve_lifecycle_target(&TargetSelector::new(&root)).unwrap();
        assert_eq!(lifecycle.kind, TargetKind::ProjectLocal);
        assert!(lifecycle.layout.is_some());
    }
}
