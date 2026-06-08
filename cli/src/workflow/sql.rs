//! Shared workflow SQL execution helpers.

use std::{env, fs, path::Path};

use kalam_client::{AuthProvider, HttpVersion, KalamLinkClient, QueryResponse};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    sql_batch,
    workflow::{
        dev::server::local_server_root_password, project::resolve::ResolvedEnvironment,
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

fn resolve_workflow_auth_provider(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<AuthProvider> {
    if ctx.config.dev.auto_start_db {
        if let Some(password) = local_server_root_password(&ctx.project_root)? {
            return Ok(AuthProvider::system_user_auth(password));
        }
    }

    let env_user = env::var("KALAM_USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| env::var("KALAMDB_USER").ok().filter(|value| !value.trim().is_empty()));
    let env_password = env::var("KALAM_PASSWORD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| env::var("KALAMDB_PASSWORD").ok().filter(|value| !value.trim().is_empty()));
    if let (Some(user), Some(password)) = (env_user, env_password) {
        return Ok(AuthProvider::basic_auth(user, password));
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
