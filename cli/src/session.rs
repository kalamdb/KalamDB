//! CLI session state management
//!
//! **Implements T084**: CLISession state with kalam-client integration
//! **Implements T091-T093**: Interactive readline loop with command execution
//! **Implements T114a**: Loading indicator for long-running queries
//!
//! Manages the connection to KalamDB server and execution state throughout
//! the CLI session lifetime.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    config::CLIConfiguration, error::Result, formatter::OutputFormatter, parser::CommandParser,
    update_check::UpdateAvailability,
};
use clap::ValueEnum;
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use kalam_client::{
    credentials::{CredentialStore, Credentials},
    AuthProvider, AuthRefreshCallback, ConnectionOptions, KalamLinkClient, KalamLinkError,
    KalamLinkTimeouts, TimestampFormatter,
};

pub mod auth_options;
mod batch;
mod cluster;
mod commands;
mod credentials;
mod health;
mod info;
mod interactive;
mod namespace;
pub mod oidc_browser;
pub mod oidc_device;
mod query;
mod subscriptions;
mod table_transfer;

const SPINNER_TICKS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_TICK_INTERVAL_MS: u64 = 80;

/// Output format for query results
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

/// Cluster node information for CLI display
#[derive(Debug, Clone)]
struct ClusterNodeDisplay {
    node_id: u64,
    role: String,
    status: String,
    api_addr: String,
    is_self: bool,
    is_leader: bool,
    version: Option<String>,
}

/// Cluster information for CLI display
#[derive(Debug, Clone)]
struct ClusterInfoDisplay {
    is_cluster_mode: bool,
    cluster_name: String,
    current_node: Option<ClusterNodeDisplay>,
    nodes: Vec<ClusterNodeDisplay>,
}

/// CLI session state
pub struct CLISession {
    /// KalamDB client
    client: KalamLinkClient,

    /// Command parser
    parser: CommandParser,

    /// Output formatter
    formatter: OutputFormatter,

    /// Timestamp formatter for displaying time values
    timestamp_formatter: TimestampFormatter,

    /// CLI configuration
    config: CLIConfiguration,

    /// CLI config file path
    config_path: PathBuf,

    /// Server URL
    server_url: String,

    /// Server host (cached for prompt rendering)
    server_host: String,

    /// Output format
    format: OutputFormat,

    /// Enable colored output
    color: bool,

    /// Session is connected
    connected: bool,

    /// Current namespace for unqualified SQL statements
    current_namespace: Option<String>,

    /// Active subscription paused state
    subscription_paused: bool,

    /// Threshold for showing loading indicator (milliseconds)
    loading_threshold_ms: u64,

    /// Enable spinners/animations
    animations: bool,

    /// Authenticated user identifier
    username: String,

    /// Session start time
    connected_at: Instant,

    /// Number of queries executed in this session
    queries_executed: u64,

    /// Server version
    server_version: Option<String>,

    /// Server API version
    server_api_version: Option<String>,

    /// Server build date
    server_build_date: Option<String>,

    /// Latest CLI update available for this local binary
    update_available: Option<UpdateAvailability>,

    /// Instance name for credential management
    instance: Option<String>,

    /// Cluster name from server (for prompt display)
    cluster_name: Option<String>,

    /// Credential store for managing saved credentials
    credential_store: Option<Arc<Mutex<crate::credentials::FileCredentialStore>>>,

    /// Whether credentials were loaded from storage (vs. provided on command line)
    credentials_loaded: bool,

    /// Authentication provider (kept for HTTP API calls)
    auth: AuthProvider,

    /// Configured timeouts for operations
    #[allow(dead_code)] // Reserved for future use
    timeouts: KalamLinkTimeouts,
}

impl CLISession {
    /// Create a new CLI session with AuthProvider
    ///
    /// **Implements T120**: Create session from stored credentials
    pub async fn with_auth(
        server_url: String,
        auth: AuthProvider,
        format: OutputFormat,
        color: bool,
    ) -> Result<Self> {
        Self::with_auth_and_instance(
            server_url,
            auth,
            format,
            color,
            None,
            None,
            None,
            None,
            true,
            None,
            None,
            None,
            CLIConfiguration::default(),
            crate::config::default_config_path(),
            false,
        )
        .await
    }

    /// Create a new CLI session with AuthProvider, instance name, and credential store
    ///
    /// **Implements T121-T122**: CLI credential management commands
    #[allow(clippy::too_many_arguments)]
    pub async fn with_auth_and_instance(
        server_url: String,
        auth: AuthProvider,
        format: OutputFormat,
        color: bool,
        instance: Option<String>,
        credential_store: Option<crate::credentials::FileCredentialStore>,
        authenticated_username: Option<String>,
        loading_threshold_ms: Option<u64>,
        animations: bool,
        client_timeout: Option<Duration>,
        timeouts: Option<KalamLinkTimeouts>,
        connection_options: Option<ConnectionOptions>,
        config: CLIConfiguration,
        config_path: PathBuf,
        credentials_loaded: bool,
    ) -> Result<Self> {
        // Build kalam-client with authentication and timeouts
        let timeouts = timeouts.unwrap_or_default();
        let timeout = client_timeout.unwrap_or(timeouts.receive_timeout);

        let timestamp_formatter = connection_options
            .as_ref()
            .map(|opts| opts.create_formatter())
            .unwrap_or_else(|| ConnectionOptions::default().create_formatter());

        let credential_store = credential_store.map(|store| Arc::new(Mutex::new(store)));

        // Build client with connection options if provided
        let mut builder = KalamLinkClient::builder()
            .base_url(&server_url)
            .timeout(timeout)
            .max_retries(config.resolved_server().max_retries)
            .auth(auth.clone())
            .timeouts(timeouts.clone());

        if let Some(refresher) =
            Self::build_auth_refresher(&server_url, instance.as_deref(), credential_store.clone())
        {
            builder = builder.auth_refresher(refresher);
        }

        if let Some(opts) = connection_options {
            builder = builder.connection_options(opts);
        }

        let client = builder.build()?;

        // Try to fetch server info from health check (sanitize empty strings to None)
        fn normalize_opt_string(s: String) -> Option<String> {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }

        let (server_version, server_api_version, server_build_date, connected) =
            match client.health_check().await {
                Ok(health) => (
                    normalize_opt_string(health.version),
                    normalize_opt_string(health.api_version),
                    health.build_date.and_then(|s| {
                        let trimmed = s.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        }
                    }),
                    true,
                ),
                // Health endpoint is localhost-only; treat as connected (version info unavailable)
                Err(KalamLinkError::ServerError {
                    status_code: 403, ..
                }) => (None, None, None, true),
                Err(_e) => (None, None, None, false),
            };

        // Use provided username or extract from auth provider
        let username = if let Some(name) = authenticated_username {
            name
        } else {
            match &auth {
                AuthProvider::BasicAuth(username, _) => username.clone(),
                AuthProvider::JwtToken(_) => "jwt-user".to_string(),
                AuthProvider::None => "anonymous".to_string(),
            }
        };
        let server_host = Self::extract_host(&server_url);

        Ok(Self {
            client,
            parser: CommandParser::new(),
            formatter: OutputFormatter::new(format, color, timestamp_formatter.clone()),
            timestamp_formatter,
            config,
            config_path,
            server_url,
            server_host,
            format,
            color,
            connected,
            current_namespace: None,
            subscription_paused: false,
            loading_threshold_ms: loading_threshold_ms.unwrap_or(200),
            animations,
            username,
            connected_at: Instant::now(),
            queries_executed: 0,
            server_version,
            server_api_version,
            server_build_date,
            update_available: None,
            instance,
            cluster_name: None, // Will be fetched lazily from system.cluster
            credential_store,
            credentials_loaded,
            auth,
            timeouts,
        })
    }

    /// Build the auth-refresh callback for TOKEN_EXPIRED recovery.
    ///
    /// Returns an `AuthRefreshCallback` that uses the credential store's
    /// refresh token to obtain a fresh JWT. Called by the kalam-client
    /// `QueryExecutor` when the server responds with TOKEN_EXPIRED.
    pub fn build_auth_refresher(
        server_url: &str,
        instance: Option<&str>,
        credential_store: Option<Arc<Mutex<crate::credentials::FileCredentialStore>>>,
    ) -> Option<AuthRefreshCallback> {
        let instance = instance?.to_owned();
        let store = credential_store?;
        let url = server_url.to_owned();

        Some(Arc::new(move || {
            let instance = instance.clone();
            let url = url.clone();
            let store = Arc::clone(&store);
            Box::pin(async move {
                let (refresh_token, creds) = {
                    let s = store.lock().unwrap();
                    let creds = s.get_credentials(&instance).ok().flatten();
                    match creds {
                        Some(c) if c.can_refresh() => (c.refresh_token.clone(), Some(c)),
                        _ => (None, None),
                    }
                };

                let refresh_token = refresh_token.ok_or_else(|| {
                    KalamLinkError::AuthenticationError("No refresh token available".to_string())
                })?;
                let creds = creds.unwrap();

                let temp_client = KalamLinkClient::builder()
                    .base_url(&url)
                    .timeout(std::time::Duration::from_secs(10))
                    .build()?;

                let login_response = temp_client.refresh_access_token(&refresh_token).await?;

                // Persist refreshed credentials
                let new_creds = Credentials::with_refresh_token(
                    instance.clone(),
                    login_response.access_token.clone(),
                    login_response.user.id.to_string(),
                    login_response.expires_at.clone(),
                    creds.server_url.clone().or_else(|| Some(url.clone())),
                    login_response.refresh_token.clone().or_else(|| creds.refresh_token.clone()),
                    login_response
                        .refresh_expires_at
                        .clone()
                        .or_else(|| creds.refresh_expires_at.clone()),
                );
                if let Ok(mut s) = store.lock() {
                    let _ = s.set_credentials(&new_creds);
                }

                Ok(AuthProvider::jwt_token(login_response.access_token))
            })
        }))
    }

    pub fn print_execution_target_banner(&self) {
        let instance = self.instance.as_deref().unwrap_or("default");
        let user = match self.username.as_str() {
            "jwt-user" => "token-authenticated (user id unavailable)",
            "anonymous" => "anonymous",
            _ => self.username.as_str(),
        };

        if self.color {
            eprintln!("{} {}", "Instance:".cyan().bold(), instance.green().bold());
            eprintln!("{} {}", "Server:".cyan().bold(), self.server_url.green());
            eprintln!("{} {}", "User:".cyan().bold(), user.green().bold());
        } else {
            eprintln!("Instance: {}", instance);
            eprintln!("Server: {}", self.server_url);
            eprintln!("User: {}", user);
        }
        eprintln!();
    }

    /// Execute a SQL query with loading indicator
    ///
    /// **Implements T092**: Execute SQL via kalam-client
    /// **Implements T114a**: Show loading indicator for queries > threshold
    /// **Enhanced**: Colored output and styled timing
    pub async fn execute_input(&mut self, input: &str) -> Result<()> {
        let command = self.parser.parse_external_command(input)?;
        self.execute_command(command).await
    }

    fn extract_host(url: &str) -> String {
        let trimmed = url.trim();
        let without_scheme = trimmed
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_start_matches("ws://")
            .trim_start_matches("wss://");

        let host = without_scheme.split(['/', '?']).next().unwrap_or(without_scheme);

        if host.is_empty() {
            "localhost".to_string()
        } else {
            host.to_string()
        }
    }

    /// Get current server URL
    pub fn server_url(&self) -> &str {
        &self.server_url
    }
    /// Get the base API URL (server_url without trailing slash).
    pub(super) fn api_base(&self) -> String {
        self.server_url.trim_end_matches('/').to_string()
    }

    /// Return the value for an `Authorization` header suitable for direct HTTP calls.
    ///
    /// For `JwtToken` auth this returns `Bearer <token>`.
    /// For `BasicAuth` it returns `Basic <base64>` (the server accepts Basic on all REST endpoints).
    /// For `None` auth it returns an empty string (no header added).
    pub(super) fn authorization_header_value(&self) -> Option<String> {
        match &self.auth {
            AuthProvider::JwtToken(token) => Some(format!("Bearer {}", token)),
            AuthProvider::BasicAuth(user, pass) => {
                use base64::Engine as _;
                let encoded =
                    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", user, pass));
                Some(format!("Basic {}", encoded))
            },
            AuthProvider::None => None,
        }
    }

    /// Check if session is connected
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Set output format
    pub fn set_format(&mut self, format: OutputFormat) {
        self.format = format;
        self.formatter = OutputFormatter::new(format, self.color, self.timestamp_formatter.clone());
    }

    /// Set color mode
    pub fn set_color(&mut self, enabled: bool) {
        self.color = enabled;
        self.formatter =
            OutputFormatter::new(self.format, enabled, self.timestamp_formatter.clone());
    }

    fn create_spinner(message: &str) -> ProgressBar {
        let progress_bar = ProgressBar::new_spinner();
        progress_bar.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(SPINNER_TICKS)
                .template("{spinner:.cyan} {msg}")
                .expect("spinner template should be valid"),
        );
        progress_bar.set_message(message.to_string());
        progress_bar.enable_steady_tick(Duration::from_millis(SPINNER_TICK_INTERVAL_MS));
        progress_bar
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use kalam_client::{
        credentials::{CredentialStore, Credentials},
        SeqId,
    };
    use ntest::timeout;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::Mutex as AsyncMutex,
    };

    use super::*;
    use crate::credentials::FileCredentialStore;

    #[derive(Debug)]
    struct TestServerState {
        sql_authorization_headers: Vec<String>,
        sql_request_bodies: Vec<String>,
        refresh_authorization_headers: Vec<String>,
        health_authorization_headers: Vec<String>,
        cluster_health_authorization_headers: Vec<String>,
        health_status: u16,
        cluster_health_status: u16,
    }

    impl Default for TestServerState {
        fn default() -> Self {
            Self {
                sql_authorization_headers: Vec::new(),
                sql_request_bodies: Vec::new(),
                refresh_authorization_headers: Vec::new(),
                health_authorization_headers: Vec::new(),
                cluster_health_authorization_headers: Vec::new(),
                health_status: 200,
                cluster_health_status: 404,
            }
        }
    }

    struct TestServer {
        base_url: String,
        state: Arc<AsyncMutex<TestServerState>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl TestServer {
        async fn spawn() -> Self {
            let listener =
                TcpListener::bind("127.0.0.1:0").await.expect("bind test server listener");
            let address = listener.local_addr().expect("read local addr");
            let state = Arc::new(AsyncMutex::new(TestServerState::default()));
            let state_clone = Arc::clone(&state);

            let task = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let state = Arc::clone(&state_clone);
                    tokio::spawn(async move {
                        let _ = handle_test_connection(stream, state).await;
                    });
                }
            });

            Self {
                base_url: format!("http://{}", address),
                state,
                task,
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn handle_test_connection(
        mut stream: TcpStream,
        state: Arc<AsyncMutex<TestServerState>>,
    ) -> std::io::Result<()> {
        let request = read_http_request(&mut stream).await?;
        let authorization = request.headers.get("authorization").cloned();

        let (status_line, body) = match request.path.as_str() {
            "/v1/api/healthcheck" => {
                let status = {
                    let mut guard = state.lock().await;
                    guard
                        .health_authorization_headers
                        .push(authorization.clone().unwrap_or_default());
                    guard.health_status
                };

                match status {
                    200 => (
                        "HTTP/1.1 200 OK",
                        json!({
                            "status": "healthy",
                            "version": "test",
                            "api_version": "v1",
                            "build_date": null
                        })
                        .to_string(),
                    ),
                    403 => (
                        "HTTP/1.1 403 Forbidden",
                        json!({ "message": "localhost only" }).to_string(),
                    ),
                    code => (
                        "HTTP/1.1 500 Internal Server Error",
                        json!({ "message": format!("unexpected status {code}") }).to_string(),
                    ),
                }
            },
            "/v1/api/cluster/health" => {
                let status = {
                    let mut guard = state.lock().await;
                    guard
                        .cluster_health_authorization_headers
                        .push(authorization.clone().unwrap_or_default());
                    guard.cluster_health_status
                };

                match status {
                    200 => (
                        "HTTP/1.1 200 OK",
                        json!({
                            "status": "healthy",
                            "version": "test",
                            "build_date": "2026-05-04T00:00:00Z",
                            "is_cluster_mode": true,
                            "cluster_id": "test-cluster",
                            "node_id": 0,
                            "is_leader": true,
                            "total_groups": 1,
                            "groups_leading": 1,
                            "current_term": 1,
                            "last_applied": 1,
                            "millis_since_quorum_ack": 0,
                            "nodes": [{
                                "node_id": 0,
                                "role": "leader",
                                "status": "active",
                                "api_addr": "http://127.0.0.1:2900",
                                "is_self": true,
                                "is_leader": true,
                                "replication_lag": null,
                                "catchup_progress_pct": null,
                                "hostname": "localhost",
                                "memory_usage_mb": 64,
                                "cpu_usage_percent": 0.25,
                                "uptime_seconds": 42,
                                "uptime_human": "42s"
                            }]
                        })
                        .to_string(),
                    ),
                    403 => (
                        "HTTP/1.1 403 Forbidden",
                        json!({ "message": "localhost only" }).to_string(),
                    ),
                    _ => ("HTTP/1.1 404 Not Found", "Not found".to_string()),
                }
            },
            "/v1/api/sql" => {
                if let Some(header) = authorization.clone() {
                    let mut guard = state.lock().await;
                    guard.sql_authorization_headers.push(header.clone());
                    guard.sql_request_bodies.push(request.body.clone());
                    match header.as_str() {
                        "Bearer expired-token" => (
                            "HTTP/1.1 401 Unauthorized",
                            json!({
                                "status": "error",
                                "error": {
                                    "code": "TOKEN_EXPIRED",
                                    "message": "Token expired"
                                }
                            })
                            .to_string(),
                        ),
                        "Bearer fresh-token" => (
                            "HTTP/1.1 200 OK",
                            json!({
                                "status": "success",
                                "results": [{
                                    "schema": [{
                                        "name": "count",
                                        "data_type": "BigInt",
                                        "index": 0
                                    }],
                                    "rows": [["1"]],
                                    "row_count": 1
                                }],
                                "took": 1.0
                            })
                            .to_string(),
                        ),
                        _ => ("HTTP/1.1 401 Unauthorized", "Unauthorized".to_string()),
                    }
                } else {
                    ("HTTP/1.1 401 Unauthorized", "Missing authorization".to_string())
                }
            },
            "/v1/api/auth/refresh" => {
                if let Some(header) = authorization.clone() {
                    state.lock().await.refresh_authorization_headers.push(header.clone());
                    if header == "Bearer refresh-token" {
                        (
                            "HTTP/1.1 200 OK",
                            json!({
                                "user": {
                                    "id": "admin",
                                    "role": "dba",
                                    "email": null,
                                    "created_at": "2026-03-17T00:00:00Z",
                                    "updated_at": "2026-03-17T00:00:00Z"
                                },
                                "admin_ui_access": true,
                                "expires_at": "2099-01-01T00:00:00Z",
                                "access_token": "fresh-token",
                                "refresh_token": "fresh-refresh-token",
                                "refresh_expires_at": "2099-02-01T00:00:00Z"
                            })
                            .to_string(),
                        )
                    } else {
                        ("HTTP/1.1 401 Unauthorized", "Invalid refresh token".to_string())
                    }
                } else {
                    ("HTTP/1.1 401 Unauthorized", "Missing authorization".to_string())
                }
            },
            _ => ("HTTP/1.1 404 Not Found", "Not found".to_string()),
        };

        let response = format!(
            "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: \
             close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await
    }

    struct TestHttpRequest {
        path: String,
        headers: HashMap<String, String>,
        body: String,
    }

    async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<TestHttpRequest> {
        let mut buffer = Vec::new();
        let mut temp = [0_u8; 1024];
        let mut header_end = None;
        let mut content_length = 0_usize;

        loop {
            let bytes_read = stream.read(&mut temp).await?;
            if bytes_read == 0 {
                break;
            }
            buffer.extend_from_slice(&temp[..bytes_read]);

            if header_end.is_none() {
                if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = Some(position + 4);
                    let header_text = String::from_utf8_lossy(&buffer[..position]);
                    for line in header_text.lines().skip(1) {
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                content_length = value.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
            }

            if let Some(end) = header_end {
                if buffer.len() >= end + content_length {
                    break;
                }
            }
        }

        let header_end = header_end.expect("request should include headers");
        let header_text = String::from_utf8_lossy(&buffer[..header_end - 4]);
        let mut lines = header_text.lines();
        let request_line = lines.next().expect("request line");
        let path = request_line.split_whitespace().nth(1).expect("request path").to_string();

        let mut headers = HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }

        let body = String::from_utf8_lossy(&buffer[header_end..]).to_string();

        Ok(TestHttpRequest {
            path,
            headers,
            body,
        })
    }

    fn create_temp_store() -> (FileCredentialStore, TempDir) {
        let temp_dir = TempDir::new().expect("create temp dir");
        let store_path = temp_dir.path().join("credentials.toml");
        let store = FileCredentialStore::with_path(store_path).expect("create credential store");
        (store, temp_dir)
    }

    #[test]
    fn test_output_format() {
        let format = OutputFormat::Table;
        assert!(matches!(format, OutputFormat::Table));
    }

    #[test]
    fn test_extract_subscribe_options_with_semicolon() {
        // Test that semicolon is properly trimmed from subscription queries
        let (sql, _) = CLISession::extract_subscribe_options("SELECT * FROM table;");
        assert_eq!(sql, "SELECT * FROM table");

        let (sql, _) = CLISession::extract_subscribe_options("SELECT * FROM table ;");
        assert_eq!(sql, "SELECT * FROM table");

        let (sql, _) = CLISession::extract_subscribe_options("SELECT * FROM table");
        assert_eq!(sql, "SELECT * FROM table");
    }

    #[test]
    fn test_extract_subscribe_options_with_options_and_semicolon() {
        // Test OPTIONS parsing with semicolon
        let (sql, options) =
            CLISession::extract_subscribe_options("SELECT * FROM table OPTIONS (last_rows=50);");
        assert_eq!(sql, "SELECT * FROM table");
        let options = options.expect("options should parse");
        assert_eq!(options.last_rows, Some(50));
    }

    #[test]
    fn test_extract_subscribe_options_with_combined_options() {
        let (sql, options) = CLISession::extract_subscribe_options(
            "SELECT * FROM table OPTIONS (last_rows=20, batch_size=5, from=42);",
        );

        assert_eq!(sql, "SELECT * FROM table");
        let options = options.expect("options should parse");
        assert_eq!(options.last_rows, Some(20));
        assert_eq!(options.batch_size, Some(5));
        assert_eq!(options.from, Some(SeqId::from(42)));
    }

    #[test]
    fn test_extract_subscribe_options_accepts_from_seq_id_alias() {
        let (sql, options) = CLISession::extract_subscribe_options(
            "SELECT * FROM table OPTIONS (batch_size=10, from_seq_id=99);",
        );

        assert_eq!(sql, "SELECT * FROM table");
        let options = options.expect("options should parse");
        assert_eq!(options.batch_size, Some(10));
        assert_eq!(options.from, Some(SeqId::from(99)));
    }

    #[test]
    fn test_extract_subscription_config_uses_shared_subscription_types() {
        let response: kalam_client::QueryResponse = serde_json::from_value(json!({
            "status": "success",
            "results": [{
                "schema": [
                    {"name": "status", "data_type": "Text", "index": 0},
                    {"name": "ws_url", "data_type": "Text", "index": 1},
                    {"name": "subscription", "data_type": "Json", "index": 2},
                    {"name": "message", "data_type": "Text", "index": 3}
                ],
                "rows": [[
                    "subscription_required",
                    "ws://localhost:2900/v1/ws",
                    {
                        "id": "sub-1",
                        "sql": "SELECT * FROM chat.messages",
                        "options": {"last_rows": 10}
                    },
                    "Subscription created. Connect to ws_url to receive updates."
                ]],
                "row_count": 1
            }]
        }))
        .expect("subscription response should deserialize");

        let (config, message) = CLISession::extract_subscription_config(&response)
            .expect("subscription config extraction should succeed")
            .expect("subscription response should produce a config");

        assert_eq!(config.id, "sub-1");
        assert_eq!(config.sql, "SELECT * FROM chat.messages");
        assert_eq!(config.ws_url.as_deref(), Some("ws://localhost:2900/v1/ws"));
        assert_eq!(config.options.and_then(|options| options.last_rows), Some(10));
        assert_eq!(
            message.as_deref(),
            Some("Subscription created. Connect to ws_url to receive updates.")
        );
    }

    #[test]
    fn test_format_row_returns_compact_json_without_newlines() {
        let mut row = kalam_client::RowData::new();
        row.insert("message".to_string(), kalam_client::KalamCellValue::text("hello"));
        row.insert("count".to_string(), kalam_client::KalamCellValue::int(5));

        let formatted = CLISession::format_row(&row);

        assert!(!formatted.contains('\n'));
        assert!(!formatted.contains("  "));
        assert!(formatted.starts_with('{'));
        assert!(formatted.ends_with('}'));
        assert!(formatted.contains("\"message\":\"hello\""));
        assert!(formatted.contains("\"count\":5"));
    }

    #[test]
    fn test_parse_describe_target_supports_namespace_and_semicolon() {
        let (namespace, table_name) = CLISession::parse_describe_target("chat.messages;").unwrap();
        assert_eq!(namespace.as_deref(), Some("chat"));
        assert_eq!(table_name, "messages");

        let (namespace, table_name) =
            CLISession::parse_describe_target("\"chat\".\"message logs\"").unwrap();
        assert_eq!(namespace.as_deref(), Some("chat"));
        assert_eq!(table_name, "message logs");
    }

    #[test]
    fn test_parse_namespace_switch_statements() {
        assert_eq!(
            CLISession::parse_namespace_switch("USE NAMESPACE chat;"),
            Some("chat".to_string())
        );
        assert_eq!(
            CLISession::parse_namespace_switch("SET NAMESPACE \"team chat\""),
            Some("team chat".to_string())
        );
        assert_eq!(CLISession::parse_namespace_switch("USE billing"), Some("billing".to_string()));
        assert_eq!(CLISession::parse_namespace_switch("USE NAMESPACE chat; SELECT 1"), None);
    }

    #[test]
    fn test_split_batch_statements_preserves_execute_as_and_strings() {
        let statements = CLISession::split_batch_statements(
            "CREATE TABLE demo.t (id BIGINT PRIMARY KEY, msg TEXT);\nEXECUTE AS USER 'alice' (INSERT INTO demo.t (id, msg) VALUES (1, 'hello;still string'));\nSELECT * FROM demo.t;",
        );

        assert_eq!(statements.len(), 3);
        assert!(statements[1].starts_with("EXECUTE AS USER 'alice'"));
        assert!(statements[1].contains("hello;still string"));
    }

    #[test]
    fn test_batch_table_readiness_target_detects_create_user_table() {
        assert_eq!(
            CLISession::batch_table_readiness_target(
                "CREATE USER TABLE demo.activity_feed (id BIGINT PRIMARY KEY)",
            ),
            Some("demo.activity_feed".to_string())
        );
        assert_eq!(
            CLISession::batch_table_readiness_target(
                "EXECUTE AS USER 'demo-user' (INSERT INTO demo.activity_feed (id) VALUES (1))",
            ),
            None
        );
    }

    #[tokio::test]
    #[timeout(5000)]
    async fn test_execute_input_applies_current_namespace_to_followup_queries() {
        let server = TestServer::spawn().await;

        let mut session = CLISession::with_auth_and_instance(
            server.base_url.clone(),
            AuthProvider::jwt_token("fresh-token".to_string()),
            OutputFormat::Json,
            false,
            None,
            None,
            Some("admin".to_string()),
            None,
            false,
            None,
            None,
            None,
            CLIConfiguration::default(),
            crate::config::default_config_path(),
            false,
        )
        .await
        .expect("session should initialize");

        session
            .execute_input("USE NAMESPACE chat;")
            .await
            .expect("use namespace should succeed");
        session
            .execute_input("SELECT * FROM messages;")
            .await
            .expect("follow-up query should succeed");

        assert_eq!(session.current_namespace.as_deref(), Some("chat"));

        let state = server.state.lock().await;
        assert_eq!(state.sql_request_bodies.len(), 2);
        assert!(
            state.sql_request_bodies[1].contains("\"namespace_id\":\"chat\""),
            "expected namespace_id form field in request body: {}",
            state.sql_request_bodies[1]
        );
    }

    #[tokio::test]
    #[timeout(5000)]
    async fn test_health_check_does_not_fall_back_to_authenticated_sql() {
        let server = TestServer::spawn().await;
        {
            let mut state = server.state.lock().await;
            state.health_status = 403;
            state.cluster_health_status = 403;
        }

        let mut session = CLISession::with_auth_and_instance(
            server.base_url.clone(),
            AuthProvider::jwt_token("fresh-token".to_string()),
            OutputFormat::Table,
            false,
            None,
            None,
            Some("admin".to_string()),
            None,
            true,
            None,
            None,
            None,
            CLIConfiguration::default(),
            crate::config::default_config_path(),
            false,
        )
        .await
        .expect("create session");

        session.health_check().await.expect("health check should succeed");

        let state = server.state.lock().await;
        assert!(state.sql_authorization_headers.is_empty());
        assert_eq!(state.health_authorization_headers, vec![String::new(), String::new()]);
        assert_eq!(state.cluster_health_authorization_headers, vec![String::new()]);
    }

    #[tokio::test]
    #[timeout(5000)]
    async fn test_build_auth_refresher_refreshes_and_persists_tokens() {
        let server = TestServer::spawn().await;
        let (mut store, _temp_dir) = create_temp_store();
        let creds = Credentials::with_refresh_token(
            "local".to_string(),
            "expired-token".to_string(),
            "admin".to_string(),
            "2000-01-01T00:00:00Z".to_string(),
            Some(server.base_url.clone()),
            Some("refresh-token".to_string()),
            Some("2099-01-01T00:00:00Z".to_string()),
        );
        store.set_credentials(&creds).expect("store initial credentials");

        let refresher = CLISession::build_auth_refresher(
            &server.base_url,
            Some("local"),
            Some(Arc::new(Mutex::new(store))),
        )
        .expect("build auth refresher");

        let refreshed_auth = refresher().await.expect("refresh should succeed");
        assert!(matches!(
            refreshed_auth,
            AuthProvider::JwtToken(token) if token == "fresh-token"
        ));

        let state = server.state.lock().await;
        assert_eq!(state.refresh_authorization_headers, vec!["Bearer refresh-token".to_string()]);
    }

    #[tokio::test]
    #[timeout(5000)]
    async fn test_execute_refreshes_expired_token_during_active_session() {
        let server = TestServer::spawn().await;
        let (mut store, _temp_dir) = create_temp_store();
        let creds = Credentials::with_refresh_token(
            "local".to_string(),
            "expired-token".to_string(),
            "admin".to_string(),
            "2000-01-01T00:00:00Z".to_string(),
            Some(server.base_url.clone()),
            Some("refresh-token".to_string()),
            Some("2099-01-01T00:00:00Z".to_string()),
        );
        store.set_credentials(&creds).expect("store initial credentials");

        let mut session = CLISession::with_auth_and_instance(
            server.base_url.clone(),
            AuthProvider::jwt_token("expired-token".to_string()),
            OutputFormat::Json,
            false,
            Some("local".to_string()),
            Some(store),
            Some("admin".to_string()),
            Some(0),
            false,
            Some(Duration::from_secs(2)),
            None,
            None,
            CLIConfiguration::default(),
            crate::config::default_config_path(),
            true,
        )
        .await
        .expect("create session");

        session
            .execute("SELECT count(*) FROM system.stats")
            .await
            .expect("query should succeed after token refresh");

        let stored = session
            .credential_store
            .as_ref()
            .expect("credential store available")
            .lock()
            .unwrap()
            .get_credentials("local")
            .expect("load refreshed credentials")
            .expect("stored credentials present");
        assert_eq!(stored.jwt_token, "fresh-token");
        assert_eq!(stored.refresh_token.as_deref(), Some("fresh-refresh-token"));

        let state = server.state.lock().await;
        assert_eq!(
            state.sql_authorization_headers,
            vec![
                "Bearer expired-token".to_string(),
                "Bearer fresh-token".to_string()
            ]
        );
        assert_eq!(state.refresh_authorization_headers, vec!["Bearer refresh-token".to_string()]);
    }
}
