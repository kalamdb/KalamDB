//! Startup validation for `kalam dev`.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use kalam_client::{
    credentials::{CredentialStore, Credentials},
    AuthProvider, KalamLinkClient, LoginResponse,
};

use url::Url;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::ProgressTaskStatus,
    workflow::{
        dev::{
            logs::ServiceLogSource,
            server::{
                ensure_local_server_binary, local_server_root_password, server_already_ready,
            },
            watch::schema_watch_path,
        },
        display_project_path,
        project::config::SchemaMode,
        project::resolve::{
            credential_instance_for_env, resolve_kalam_profile, ResolvedEnvironment,
        },
        sql::{is_loopback_server_url, resolve_workflow_auth_provider},
        WorkflowContext,
    },
    FileCredentialStore,
};

pub struct DevPrecheckReport {
    pub environment: ResolvedEnvironment,
    pub watch_enabled: bool,
    pub local_server_reused: bool,
}

pub async fn run_dev_prechecks(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<DevPrecheckReport> {
    let environment = ctx.resolved_environment()?;
    Url::parse(&environment.url).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "precheck failed: invalid environment URL '{}': {error}",
            environment.url
        ))
    })?;
    output
        .service_log(server_source, format!("precheck: environment ready at {}", environment.url));
    output.progress_task("environment", ProgressTaskStatus::Succeeded, "Environment ready");

    ensure_schema_source(ctx, output, server_source)?;
    ensure_generated_targets(ctx, output, server_source)?;
    ensure_migrations_dir(ctx, output, server_source)?;
    ensure_kalam_gitignore(ctx, output, server_source)?;
    ensure_cli_log_dir(ctx, output, server_source)?;

    let watch_enabled = schema_watch_path(&ctx.project_root, &ctx.config).is_some();
    if let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) {
        output.service_log(
            server_source,
            format!(
                "precheck: schema watch enabled for {}",
                display_project_path(&ctx.project_root, &path)
            ),
        );
    } else {
        output.service_log(server_source, "precheck: schema watch disabled");
    }

    let local_server_reused = if ctx.config.dev.auto_start_db {
        if server_already_ready(&environment.url).await {
            output.service_log(
                server_source,
                format!("precheck: local server reachable at {}", environment.url),
            );
            true
        } else {
            let binary = ensure_local_server_binary(ctx.use_color, output, server_source).await?;
            output.service_log(
                server_source,
                format!("precheck: local server binary ready at {}", binary.display()),
            );
            false
        }
    } else {
        if !server_already_ready(&environment.url).await {
            return Err(CLIError::ConfigurationError(format!(
                "precheck failed: could not reach KalamDB server at {}",
                environment.url
            )));
        }
        output.service_log(
            server_source,
            format!("precheck: remote server reachable at {}", environment.url),
        );
        false
    };

    if ctx.config.dev.auto_start_db && local_server_reused {
        ensure_local_dev_authentication_ready(ctx, &environment, output, server_source).await?;
    } else if !ctx.config.dev.auto_start_db {
        ensure_authentication_ready(ctx, &environment, output, server_source).await?;
    }

    Ok(DevPrecheckReport {
        environment,
        watch_enabled,
        local_server_reused,
    })
}

fn ensure_schema_source(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    if !matches!(ctx.config.schema.mode, SchemaMode::Sql) {
        return Ok(());
    }

    let path = ctx.config.schema_source_path(&ctx.project_root).ok_or_else(|| {
        CLIError::ConfigurationError("precheck failed: schema.path is not configured".into())
    })?;

    if !path.is_file() {
        return Err(CLIError::ConfigurationError(format!(
            "precheck failed: schema source missing at {}",
            display_project_path(&ctx.project_root, &path)
        )));
    }

    output.service_log(
        server_source,
        format!(
            "precheck: schema source found at {}",
            display_project_path(&ctx.project_root, &path)
        ),
    );
    output.progress_task("schema-source", ProgressTaskStatus::Succeeded, "Schema source found");
    Ok(())
}

fn ensure_generated_targets(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    if !ctx.config.dev.generate_types {
        return Ok(());
    }

    let mut ready_count = 0usize;
    for target in ctx.config.schema.targets.values() {
        let output_path = ctx.project_root.join(&target.output);
        let parent = output_path.parent().ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "precheck failed: invalid schema target output '{}'",
                display_project_path(&ctx.project_root, &output_path)
            ))
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            CLIError::FileError(format!(
                "precheck failed: could not create schema target directory '{}': {error}",
                parent.display()
            ))
        })?;
        output.service_log(
            server_source,
            format!(
                "precheck: target directory ready at {}",
                display_project_path(&ctx.project_root, parent)
            ),
        );
        ready_count += 1;
    }

    if ready_count > 0 {
        output.progress_task("target-dir", ProgressTaskStatus::Succeeded, "Target directory ready");
    }

    Ok(())
}

fn ensure_migrations_dir(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    let migrations_dir: PathBuf = ctx.config.migrations_dir(&ctx.project_root);
    fs::create_dir_all(&migrations_dir).map_err(|error| {
        CLIError::FileError(format!(
            "precheck failed: could not create migrations directory '{}': {error}",
            display_project_path(&ctx.project_root, &migrations_dir)
        ))
    })?;
    output.service_log(
        server_source,
        format!(
            "precheck: migrations directory ready at {}",
            display_project_path(&ctx.project_root, &migrations_dir)
        ),
    );
    output.progress_task(
        "migrations-dir",
        ProgressTaskStatus::Succeeded,
        migrations_ready_message(&ctx.project_root, &migrations_dir),
    );
    Ok(())
}

fn ensure_kalam_gitignore(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    let gitignore_path = ctx.config.ensure_kalam_gitignore(&ctx.project_root)?;
    output.service_log(
        server_source,
        format!(
            "precheck: KalamDB state .gitignore ready at {}",
            display_project_path(&ctx.project_root, &gitignore_path)
        ),
    );
    Ok(())
}

fn ensure_cli_log_dir(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    let log_dir = ctx.config.ensure_cli_log_dir(&ctx.project_root)?;
    output.service_log(
        server_source,
        format!(
            "precheck: KalamDB CLI log directory ready at {}",
            display_project_path(&ctx.project_root, &log_dir)
        ),
    );
    Ok(())
}

pub async fn ensure_authentication_ready(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    let profile = resolve_kalam_profile(&ctx.project_root)?;
    if let Err(detail) = verify_workflow_auth(ctx, environment).await {
        return Err(CLIError::ConfigurationError(auth_guidance_message(
            &ctx.project_root,
            profile.as_deref(),
            &detail,
        )));
    }

    output.service_log(
        server_source,
        format!("precheck: authentication ready for {}", environment.url),
    );
    output.progress_task("authentication", ProgressTaskStatus::Succeeded, "Authentication ready");
    Ok(())
}

pub async fn ensure_local_dev_authentication_ready(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<()> {
    match verify_workflow_auth(ctx, environment).await {
        Ok(()) => {
            output.service_log(
                server_source,
                format!("precheck: authentication ready for {}", environment.url),
            );
            output.progress_task(
                "authentication",
                ProgressTaskStatus::Succeeded,
                "Authentication ready",
            );
            return Ok(());
        },
        Err(detail) => output.service_log(
            server_source,
            format!("precheck: saved local dev authentication is not ready: {detail}"),
        ),
    }

    let profile = local_dev_auth_profile(ctx, environment)?;
    let password = local_dev_root_password(ctx, environment)?;
    output.service_log(
        server_source,
        format!("precheck: logging into local KalamDB server as root for profile '{profile}'"),
    );
    let login = login_local_root(&environment.url, &password).await.map_err(|detail| {
        CLIError::ConfigurationError(auth_guidance_message(
            &ctx.project_root,
            Some(&profile),
            &detail,
        ))
    })?;
    save_local_dev_credentials(&profile, &environment.url, &login)?;
    output.service_log(
        server_source,
        format!("precheck: saved local dev credentials for profile '{profile}'"),
    );

    verify_jwt_auth(&environment.url, &login.access_token).await.map_err(|detail| {
        CLIError::ConfigurationError(auth_guidance_message(
            &ctx.project_root,
            Some(&profile),
            &detail,
        ))
    })?;

    output.progress_task("authentication", ProgressTaskStatus::Succeeded, "Authentication ready");
    Ok(())
}

async fn verify_workflow_auth(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> std::result::Result<(), String> {
    let auth =
        resolve_workflow_auth_provider(ctx, environment).map_err(|error| error.to_string())?;

    match auth {
        AuthProvider::JwtToken(token) => verify_jwt_auth(&environment.url, &token).await,
        AuthProvider::BasicAuth(user, password) => {
            verify_basic_auth(&environment.url, &user, &password).await
        },
        AuthProvider::None => Err("no saved CLI authentication profile is configured".to_string()),
    }
}

fn local_dev_auth_profile(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<String> {
    Ok(resolve_kalam_profile(&ctx.project_root)?
        .unwrap_or_else(|| credential_instance_for_env(&environment.name)))
}

fn local_dev_root_password(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<String> {
    if let Some(password) = local_server_root_password(&ctx.project_root, &ctx.config)? {
        return Ok(password);
    }

    if is_loopback_server_url(&environment.url) {
        return Ok("mypass".to_string());
    }

    Err(CLIError::ConfigurationError(auth_guidance_message(
        &ctx.project_root,
        resolve_kalam_profile(&ctx.project_root)?.as_deref(),
        "local dev root password was not found",
    )))
}

async fn login_local_root(
    server_url: &str,
    password: &str,
) -> std::result::Result<LoginResponse, String> {
    let client = KalamLinkClient::builder()
        .base_url(server_url)
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| format!("failed to create login client: {error}"))?;
    client
        .login("root", password)
        .await
        .map_err(|error| format!("local root login failed: {error}"))
}

fn save_local_dev_credentials(
    profile: &str,
    server_url: &str,
    login: &LoginResponse,
) -> Result<()> {
    let mut store = FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to open credentials store: {error}"))
    })?;
    let credentials = local_dev_credentials_from_login(profile, server_url, login);
    store.set_credentials(&credentials).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to save local dev credentials for profile '{profile}': {error}"
        ))
    })
}

fn local_dev_credentials_from_login(
    profile: &str,
    server_url: &str,
    login: &LoginResponse,
) -> Credentials {
    Credentials::with_refresh_token(
        profile.to_string(),
        login.access_token.clone(),
        login.user.id.to_string(),
        login.expires_at.clone(),
        Some(server_url.to_string()),
        login.refresh_token.clone(),
        login.refresh_expires_at.clone(),
    )
    .with_identity_metadata(login.user.name.clone(), login.user.email.clone())
}

async fn verify_jwt_auth(server_url: &str, token: &str) -> std::result::Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| format!("failed to create HTTP client: {error}"))?;
    let response = client
        .get(format!("{}/v1/api/auth/me", server_url.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("failed to reach auth endpoint: {error}"))?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("authentication check failed ({status}): {body}"))
}

async fn verify_basic_auth(
    server_url: &str,
    user: &str,
    password: &str,
) -> std::result::Result<(), String> {
    let client = KalamLinkClient::builder()
        .base_url(server_url)
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| format!("failed to create login client: {error}"))?;
    client
        .login(user, password)
        .await
        .map(|_| ())
        .map_err(|error| format!("login failed: {error}"))
}

fn auth_guidance_message(project_root: &Path, profile: Option<&str>, detail: &str) -> String {
    let env_path = project_root.join(".env");
    let profile_name = profile.unwrap_or("<profile>");

    format!(
        "authentication failed: {detail}. Edit {} and set `KALAM_PROFILE={profile_name}` to a CLI-saved profile, or run `kalam login --instance {profile_name}` and then update `.env` to use that profile",
        display_project_path(project_root, &env_path)
    )
}

fn migrations_ready_message(
    project_root: &std::path::Path,
    migrations_dir: &std::path::Path,
) -> String {
    format!(
        "Migrations directory ready at {}",
        display_project_path(project_root, migrations_dir)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::{
        config::CLIConfiguration,
        workflow::project::config::{
            ConnectionEnv, DevSection, KalamProjectConfig, LoggingSection, MigrationsSection,
            ProjectSection, SchemaMode, SchemaSection,
        },
    };

    #[test]
    fn migrations_ready_message_includes_folder() {
        let message = migrations_ready_message(
            std::path::Path::new("/tmp/app"),
            std::path::Path::new("/tmp/app/kalam/migrations"),
        );
        assert_eq!(message, "Migrations directory ready at kalam/migrations");
    }

    #[test]
    fn auth_guidance_message_points_to_env_and_login() {
        let message = auth_guidance_message(
            std::path::Path::new("/tmp/app"),
            Some("prod-admin"),
            "unauthorized",
        );

        assert!(message.contains(".env"));
        assert!(message.contains("KALAM_PROFILE=prod-admin"));
        assert!(message.contains("kalam login --instance prod-admin"));
        assert!(message.contains("unauthorized"));
    }

    #[test]
    fn local_dev_auth_profile_uses_env_profile_when_present() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join(".env"), "KALAM_PROFILE=kalam-dev\n").unwrap();
        let ctx = test_ctx(temp.path());
        let environment = test_environment("dev");

        let profile = local_dev_auth_profile(&ctx, &environment).unwrap();

        assert_eq!(profile, "kalam-dev");
    }

    #[test]
    fn local_dev_auth_profile_defaults_to_workflow_env() {
        let temp = tempfile::TempDir::new().unwrap();
        let ctx = test_ctx(temp.path());
        let environment = test_environment("preview");

        let profile = local_dev_auth_profile(&ctx, &environment).unwrap();

        assert_eq!(profile, "kalam-preview");
    }

    #[test]
    fn local_dev_credentials_preserve_login_refresh_metadata() {
        let login: LoginResponse = serde_json::from_value(serde_json::json!({
            "user": {
                "id": "root",
                "role": "system",
                "name": "Root User",
                "email": "root@example.com",
                "created_at": "2026-06-09T00:00:00Z",
                "updated_at": "2026-06-09T00:00:00Z"
            },
            "admin_ui_access": true,
            "expires_at": "2026-06-09T01:00:00Z",
            "access_token": "access-root",
            "refresh_token": "refresh-root",
            "refresh_expires_at": "2026-06-10T01:00:00Z"
        }))
        .unwrap();

        let credentials =
            local_dev_credentials_from_login("kalam-dev", "http://localhost:2900", &login);

        assert_eq!(credentials.instance, "kalam-dev");
        assert_eq!(credentials.jwt_token, "access-root");
        assert_eq!(credentials.user.unwrap().as_str(), "root");
        assert_eq!(credentials.name.as_deref(), Some("Root User"));
        assert_eq!(credentials.email.as_deref(), Some("root@example.com"));
        assert_eq!(credentials.server_url.as_deref(), Some("http://localhost:2900"));
        assert_eq!(credentials.refresh_token.as_deref(), Some("refresh-root"));
        assert_eq!(credentials.refresh_expires_at.as_deref(), Some("2026-06-10T01:00:00Z"));
    }

    fn test_environment(name: &str) -> ResolvedEnvironment {
        ResolvedEnvironment {
            name: name.into(),
            url: "http://localhost:2900".into(),
            namespace: "demo".into(),
            env_source: crate::workflow::project::resolve::ResolutionSource::ProjectConfig,
            url_source: crate::workflow::project::resolve::ResolutionSource::ProjectConfig,
            namespace_source: crate::workflow::project::resolve::ResolutionSource::ProjectConfig,
        }
    }

    fn test_ctx(project_root: &std::path::Path) -> WorkflowContext {
        WorkflowContext {
            project_root: project_root.to_path_buf(),
            config: KalamProjectConfig {
                project: ProjectSection {
                    name: "demo".into(),
                    default_env: "dev".into(),
                    kalam_dir: "kalam".into(),
                },
                connection: HashMap::from([(
                    "dev".into(),
                    ConnectionEnv {
                        url: "http://localhost:2900".into(),
                        namespace: "demo".into(),
                    },
                )]),
                schema: SchemaSection {
                    mode: SchemaMode::Sql,
                    path: Some("schema.sql".into()),
                    watch: false,
                    languages: Vec::new(),
                    targets: HashMap::new(),
                },
                migrations: MigrationsSection::default(),
                dev: DevSection::default(),
                logging: LoggingSection::default(),
            },
            cli_config: CLIConfiguration::default(),
            use_color: false,
            project_dir: None,
            env_override: None,
            namespace_override: None,
            url_override: None,
        }
    }
}
