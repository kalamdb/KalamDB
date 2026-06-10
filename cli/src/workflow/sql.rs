//! Shared workflow SQL execution helpers.

use std::{fs, path::Path};

use kalam_client::{
    credentials::CredentialStore, AuthProvider, HttpVersion, KalamLinkClient, QueryResponse,
};
use url::Url;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    sql_batch,
    workflow::{
        dev::server::local_server_root_password,
        project::resolve::{resolve_kalam_profile, ResolvedEnvironment},
        WorkflowContext,
    },
    FileCredentialStore,
};

pub(crate) fn build_workflow_client(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<KalamLinkClient> {
    let auth = resolve_workflow_auth_provider(ctx, environment)?;
    let mut connection_options = ctx.cli_config.to_connection_options();
    if environment.url.starts_with("http://")
        && connection_options.http_version == HttpVersion::Http2
    {
        connection_options = connection_options.with_http_version(HttpVersion::Auto);
    }

    KalamLinkClient::builder()
        .base_url(&environment.url)
        .timeout(std::time::Duration::from_secs(ctx.cli_config.resolved_server().timeout))
        .auth(auth)
        .connection_options(connection_options)
        .build()
        .map_err(CLIError::from)
}

pub(crate) async fn ensure_namespace_exists(
    client: &KalamLinkClient,
    namespace: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    output.status(format!("ensuring namespace {namespace}"));
    let sql = format!("CREATE NAMESPACE IF NOT EXISTS {namespace}");
    execute_single_statement(client, &sql, None, "namespace bootstrap").await?;
    output.status(format!("ensured namespace {namespace}"));
    Ok(())
}

pub(crate) async fn drop_namespace_if_exists(
    client: &KalamLinkClient,
    namespace: &str,
) -> Result<()> {
    let sql = format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE");
    execute_single_statement(client, &sql, None, "namespace reset").await
}

pub(crate) async fn reset_namespace_migration_state(
    client: &KalamLinkClient,
    namespace: &str,
) -> Result<()> {
    let sql = format!("DELETE FROM system.migrations WHERE namespace = {}", sql_string(namespace));
    execute_single_statement(client, &sql, None, "migration state reset").await
}

pub(crate) async fn execute_sql_file(
    client: &KalamLinkClient,
    path: &Path,
    namespace: Option<&str>,
    output: &WorkflowOutput,
    action: &str,
) -> Result<usize> {
    let sql = fs::read_to_string(path).map_err(|error| {
        CLIError::FileError(format!("failed to read SQL file '{}': {error}", path.display()))
    })?;
    output.status(format!("{action} {}", path.display()));
    execute_sql_batch(client, &sql, namespace, output, &path.display().to_string()).await
}

pub(crate) async fn execute_sql_batch(
    client: &KalamLinkClient,
    sql: &str,
    namespace: Option<&str>,
    output: &WorkflowOutput,
    source_label: &str,
) -> Result<usize> {
    let statements = sql_batch::parse_execution_batch(sql)?;
    if statements.is_empty() {
        output.detail(format!("no executable SQL statements found in {source_label}"));
        return Ok(0);
    }

    let total = statements.len();
    for (index, statement) in statements.iter().enumerate() {
        let summary = summarize_sql(statement);
        output.detail(format!(
            "executing statement {}/{} from {}: {}",
            index + 1,
            total,
            source_label,
            summary
        ));
        execute_single_statement(client, statement, namespace, &format!("Statement {}", index + 1))
            .await?;
    }

    Ok(total)
}

pub(crate) async fn execute_single_statement(
    client: &KalamLinkClient,
    sql: &str,
    namespace: Option<&str>,
    failure_prefix: &str,
) -> Result<()> {
    let response = client.execute_query(sql, None, None, namespace).await?;
    if response.success() {
        return Ok(());
    }

    Err(CLIError::ConfigurationError(format!(
        "{failure_prefix} failed: {}",
        query_failure_message(&response, "query failed")
    )))
}

pub(crate) fn query_failure_message(response: &QueryResponse, fallback: &str) -> String {
    response
        .error
        .as_ref()
        .map(|error| error.message.clone())
        .or_else(|| response.results.first().and_then(|result| result.message.clone()))
        .unwrap_or_else(|| fallback.to_string())
}

fn summarize_sql(statement: &str) -> String {
    const MAX_LEN: usize = 96;
    let normalized = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() <= MAX_LEN {
        normalized
    } else {
        format!("{}...", &normalized[..MAX_LEN])
    }
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn resolve_workflow_auth_provider(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<AuthProvider> {
    if let Some(token) = load_project_profile_token(ctx)? {
        return Ok(AuthProvider::jwt_token(token));
    }

    if ctx.config.dev.auto_start_db {
        if let Some(password) = local_server_root_password(&ctx.project_root, &ctx.config)? {
            return Ok(AuthProvider::system_user_auth(password));
        }
    }

    if ctx.config.dev.auto_start_db && is_loopback_server_url(&environment.url) {
        return Ok(AuthProvider::system_user_auth("mypass".to_string()));
    }

    if let Some(token) = ctx.cli_config.auth.as_ref().and_then(|auth| auth.jwt_token.clone()) {
        return Ok(AuthProvider::jwt_token(token));
    }

    let store = FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to open credentials store: {error}"))
    })?;
    if let Some(credentials) =
        store.get_workflow_env_credentials(&environment.name).map_err(|error| {
            CLIError::ConfigurationError(format!("failed to load workflow credentials: {error}"))
        })?
    {
        return Ok(AuthProvider::jwt_token(credentials.jwt_token));
    }

    Ok(AuthProvider::none())
}

fn load_project_profile_token(ctx: &WorkflowContext) -> Result<Option<String>> {
    let Some(profile) = resolve_kalam_profile(&ctx.project_root)? else {
        return Ok(None);
    };

    let store = FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to open credentials store: {error}"))
    })?;
    let credentials = store.get_credentials(&profile).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to load credentials for profile '{profile}': {error}"
        ))
    })?;

    let credentials = credentials.ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "profile '{profile}' from .env was not found in ~/.kalam credentials; run `kalam login --instance {profile}`"
        ))
    })?;

    Ok(Some(credentials.jwt_token))
}

pub(crate) fn is_loopback_server_url(server_url: &str) -> bool {
    Url::parse(server_url)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| {
                host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use super::*;
    use crate::{
        config::CLIConfiguration,
        workflow::project::config::{
            ConnectionEnv, DevSection, KalamProjectConfig, LoggingSection, MigrationsSection,
            ProjectSection, SchemaMode, SchemaSection, SchemaTarget,
        },
    };
    use kalam_client::credentials::{CredentialStore, Credentials};
    use tempfile::TempDir;

    #[test]
    fn workflow_auth_prefers_project_profile_over_local_password() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);

        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join("kalam/server")).unwrap();
        fs::write(project_root.join(".env"), "KALAM_PROFILE=kalam-dev\n").unwrap();
        fs::write(
            project_root.join("kalam/server/server.toml"),
            "[auth]\nroot_password = \"mypass\"\n",
        )
        .unwrap();

        let mut store = FileCredentialStore::new().unwrap();
        store
            .set_credentials(&Credentials::new("kalam-dev".into(), "jwt-from-profile".into()))
            .unwrap();

        let ctx = WorkflowContext {
            project_root: project_root.clone(),
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
                    languages: vec!["typescript".into()],
                    targets: HashMap::from([(
                        "typescript".into(),
                        SchemaTarget {
                            output: "src/generated/kalam.ts".into(),
                        },
                    )]),
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
        };
        let environment = ResolvedEnvironment {
            name: "dev".into(),
            url: "http://localhost:2900".into(),
            namespace: "demo".into(),
            env_source: crate::workflow::project::resolve::ResolutionSource::ProjectConfig,
            url_source: crate::workflow::project::resolve::ResolutionSource::ProjectConfig,
            namespace_source: crate::workflow::project::resolve::ResolutionSource::ProjectConfig,
        };

        let auth = resolve_workflow_auth_provider(&ctx, &environment).unwrap();

        match auth {
            AuthProvider::JwtToken(token) => assert_eq!(token, "jwt-from-profile"),
            other => panic!("expected jwt auth, got {other:?}"),
        }

        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}
