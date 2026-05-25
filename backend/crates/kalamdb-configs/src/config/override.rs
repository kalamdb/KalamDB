use std::env;

use anyhow::{anyhow, Result};

use super::{
    cluster::{ClusterConfig, PeerConfig},
    types::ServerConfig,
};

fn parse_csv_env_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn parse_env<T>(name: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
{
    match env::var(name) {
        Ok(value) => value.parse().map(Some).map_err(|_| anyhow!("Invalid {name} value: {value}")),
        Err(_) => Ok(None),
    }
}

fn parse_env_with_alias<T>(name: &str, alias: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
{
    match env::var(name) {
        Ok(value) => value.parse().map(Some).map_err(|_| anyhow!("Invalid {name} value: {value}")),
        Err(_) => parse_env(alias),
    }
}

fn env_truthy(name: &str) -> Option<bool> {
    env::var(name).ok().map(|value| {
        value.eq_ignore_ascii_case("true") || value == "1" || value.eq_ignore_ascii_case("yes")
    })
}

fn env_not_false(name: &str) -> Option<bool> {
    env::var(name).ok().map(|value| {
        !(value.eq_ignore_ascii_case("false") || value == "0" || value.eq_ignore_ascii_case("no"))
    })
}

impl ServerConfig {
    /// Apply environment variable overrides for runtime configuration.
    ///
    /// Supported environment variables (T030):
    /// - KALAMDB_SERVER_HOST: Override server.host
    /// - KALAMDB_SERVER_PORT: Override server.port
    /// - KALAMDB_SERVER_PUBLIC_ORIGIN: Override server.public_origin (empty keeps Admin UI
    ///   browser-origin fallback)
    /// - KALAMDB_LOG_LEVEL: Override logging.level
    /// - KALAMDB_LOG_FORMAT: Override logging.format ("compact" | "json")
    /// - KALAMDB_LOGS_DIR: Override logging.logs_path
    /// - KALAMDB_LOG_TO_CONSOLE: Override logging.log_to_console
    /// - KALAMDB_SLOW_QUERY_THRESHOLD_MS: Override logging.slow_query_threshold_ms
    /// - KALAMDB_OTLP_ENABLED: Override logging.otlp.enabled
    /// - KALAMDB_OTLP_ENDPOINT: Override logging.otlp.endpoint
    /// - KALAMDB_OTLP_PROTOCOL: Override logging.otlp.protocol ("grpc" | "http")
    /// - KALAMDB_OTLP_SERVICE_NAME: Override logging.otlp.service_name
    /// - KALAMDB_OTLP_TIMEOUT_MS: Override logging.otlp.timeout_ms
    /// - KALAMDB_DATA_DIR: Override storage.data_path (base directory for rocksdb, storage,
    ///   snapshots)
    /// - KALAMDB_CLUSTER_ID: Override cluster.cluster_id
    /// - KALAMDB_NODE_ID: Override cluster.node_id (alias: KALAMDB_CLUSTER_NODE_ID)
    /// - KALAMDB_CLUSTER_RPC_ADDR: Override cluster.rpc_addr
    /// - KALAMDB_CLUSTER_API_ADDR: Override cluster.api_addr
    /// - KALAMDB_CLUSTER_PEERS: Override cluster.peers
    /// - KALAMDB_JWT_SECRET: Override auth.jwt_secret
    /// - KALAMDB_PG_AUTH_TOKEN: Override auth.pg_auth_token
    /// - KALAMDB_JWT_TRUSTED_ISSUERS: Override auth.jwt_trusted_issuers
    /// - KALAMDB_JWT_EXPIRY_HOURS: Override auth.jwt_expiry_hours
    /// - KALAMDB_COOKIE_SECURE: Override auth.cookie_secure
    /// - KALAMDB_ALLOW_REMOTE_SETUP: Override auth.allow_remote_setup
    /// - KALAMDB_OAUTH_AUTO_PROVISION: Override oauth.auto_provision
    ///   oauth.auto_provision (kept for backward compatibility)
    /// - KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS: Override security.cors.allowed_origins with a
    ///   comma-separated list or "*"
    /// - KALAMDB_SECURITY_TRUSTED_PROXY_RANGES: Override security.trusted_proxy_ranges
    /// - KALAMDB_RATE_LIMIT_AUTH_REQUESTS_PER_IP_PER_SEC: Override
    ///   rate_limit.max_auth_requests_per_ip_per_sec
    /// - KALAMDB_TOPIC_VISIBILITY_TIMEOUT_SECS: Override topics.visibility_timeout_secs
    ///   (alias: KALAMDB_VISIBILITY_TIMEOUT_SECS)
    /// - KALAMDB_TOPIC_DEFAULT_RETENTION_SECONDS: Override topics.default_retention_seconds
    /// - KALAMDB_TOPIC_DEFAULT_RETENTION_MAX_BYTES: Override topics.default_retention_max_bytes
    /// - KALAMDB_TOPIC_RETENTION_CHECK_INTERVAL_SECONDS: Override
    ///   topics.retention_check_interval_seconds
    /// - KALAMDB_TOPIC_RETENTION_BATCH_SIZE: Override topics.retention_batch_size
    /// - KALAMDB_WEBSOCKET_CLIENT_TIMEOUT_SECS: Override websocket.client_timeout_secs
    /// - KALAMDB_WEBSOCKET_AUTH_TIMEOUT_SECS: Override websocket.auth_timeout_secs
    /// - KALAMDB_WEBSOCKET_HEARTBEAT_INTERVAL_SECS: Override websocket.heartbeat_interval_secs
    /// - KALAMDB_RPC_TLS_ENABLED: Override rpc_tls.enabled
    /// - KALAMDB_RPC_TLS_CA_CERT: Override rpc_tls.ca_cert (file path or inline PEM)
    /// - KALAMDB_RPC_TLS_SERVER_CERT: Override rpc_tls.server_cert (file path or inline PEM)
    /// - KALAMDB_RPC_TLS_SERVER_KEY: Override rpc_tls.server_key (file path or inline PEM)
    /// - KALAMDB_RPC_TLS_REQUIRE_CLIENT_CERT: Override rpc_tls.require_client_cert
    /// - KALAMDB_SERVER_WORKERS: Override server.workers (actix-web worker threads)
    ///
    /// Environment variables take precedence over server.toml values (T031)
    pub fn apply_env_overrides(&mut self) -> Result<()> {
        self.apply_server_env_overrides()?;
        self.apply_topic_env_overrides()?;
        self.apply_logging_env_overrides()?;
        self.apply_auth_env_overrides()?;
        self.apply_security_env_overrides()?;
        self.apply_websocket_env_overrides()?;
        self.apply_storage_env_overrides();
        self.apply_cluster_env_overrides()?;
        self.apply_rpc_tls_env_overrides();
        Ok(())
    }

    fn apply_server_env_overrides(&mut self) -> Result<()> {
        if let Some(workers) = parse_env("KALAMDB_SERVER_WORKERS")? {
            self.server.workers = workers;
        }
        if let Ok(host) = env::var("KALAMDB_SERVER_HOST") {
            self.server.host = host;
        }
        if let Some(port) = parse_env("KALAMDB_SERVER_PORT")? {
            self.server.port = port;
        }
        if let Ok(origin) = env::var("KALAMDB_SERVER_PUBLIC_ORIGIN") {
            self.server.public_origin = Some(origin);
        }
        Ok(())
    }

    fn apply_topic_env_overrides(&mut self) -> Result<()> {
        if let Some(timeout) = parse_env_with_alias(
            "KALAMDB_TOPIC_VISIBILITY_TIMEOUT_SECS",
            "KALAMDB_VISIBILITY_TIMEOUT_SECS",
        )? {
            self.topics.visibility_timeout_secs = timeout;
        }
        if let Some(seconds) = parse_env("KALAMDB_TOPIC_DEFAULT_RETENTION_SECONDS")? {
            self.topics.default_retention_seconds = seconds;
        }
        if let Some(max_bytes) = parse_env("KALAMDB_TOPIC_DEFAULT_RETENTION_MAX_BYTES")? {
            self.topics.default_retention_max_bytes = max_bytes;
        }
        if let Some(interval) = parse_env("KALAMDB_TOPIC_RETENTION_CHECK_INTERVAL_SECONDS")? {
            self.topics.retention_check_interval_seconds = interval;
        }
        if let Some(batch_size) = parse_env("KALAMDB_TOPIC_RETENTION_BATCH_SIZE")? {
            self.topics.retention_batch_size = batch_size;
        }
        Ok(())
    }

    fn apply_logging_env_overrides(&mut self) -> Result<()> {
        if let Ok(level) = env::var("KALAMDB_LOG_LEVEL") {
            self.logging.level = level;
        }
        if let Ok(format) = env::var("KALAMDB_LOG_FORMAT") {
            self.logging.format = format;
        }
        if let Ok(path) = env::var("KALAMDB_LOGS_DIR") {
            self.logging.logs_path = path;
        }
        if let Some(log_to_console) = env_truthy("KALAMDB_LOG_TO_CONSOLE") {
            self.logging.log_to_console = log_to_console;
        }
        if let Some(threshold) = parse_env("KALAMDB_SLOW_QUERY_THRESHOLD_MS")? {
            self.logging.slow_query_threshold_ms = threshold;
        }
        if let Some(enabled) = env_truthy("KALAMDB_OTLP_ENABLED") {
            self.logging.otlp.enabled = enabled;
        }
        if let Ok(endpoint) = env::var("KALAMDB_OTLP_ENDPOINT") {
            self.logging.otlp.endpoint = endpoint;
        }
        if let Ok(protocol) = env::var("KALAMDB_OTLP_PROTOCOL") {
            self.logging.otlp.protocol = protocol;
        }
        if let Ok(service_name) = env::var("KALAMDB_OTLP_SERVICE_NAME") {
            self.logging.otlp.service_name = service_name;
        }
        if let Some(timeout_ms) = parse_env("KALAMDB_OTLP_TIMEOUT_MS")? {
            self.logging.otlp.timeout_ms = timeout_ms;
        }
        Ok(())
    }

    fn apply_auth_env_overrides(&mut self) -> Result<()> {
        if let Ok(secret) = env::var("KALAMDB_JWT_SECRET") {
            self.auth.jwt_secret = secret;
        }
        if let Ok(issuers) = env::var("KALAMDB_JWT_TRUSTED_ISSUERS") {
            self.auth.jwt_trusted_issuers = issuers;
        }
        if let Some(expiry_hours) = parse_env("KALAMDB_JWT_EXPIRY_HOURS")? {
            self.auth.jwt_expiry_hours = expiry_hours;
        }
        if let Some(cookie_secure) = env_not_false("KALAMDB_COOKIE_SECURE") {
            self.auth.cookie_secure = cookie_secure;
        }
        if let Some(allow_remote_setup) = env_truthy("KALAMDB_ALLOW_REMOTE_SETUP") {
            self.auth.allow_remote_setup = allow_remote_setup;
        }
        if let Some(auto_provision) = env_truthy("KALAMDB_OAUTH_AUTO_PROVISION")
            .or_else(|| env_truthy("KALAMDB_AUTH_AUTO_CREATE_USERS_FROM_PROVIDER"))
        {
            self.oauth.auto_provision = auto_provision;
        }
        if let Ok(token) = env::var("KALAMDB_PG_AUTH_TOKEN") {
            self.auth.pg_auth_token = Some(token);
        }
        Ok(())
    }

    fn apply_security_env_overrides(&mut self) -> Result<()> {
        if let Ok(origins) = env::var("KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS") {
            self.security.cors.allowed_origins = parse_csv_env_list(&origins);
        }
        if let Ok(ranges) = env::var("KALAMDB_SECURITY_TRUSTED_PROXY_RANGES")
            .or_else(|_| env::var("KALAMDB_TRUSTED_PROXY_RANGES"))
        {
            self.security.trusted_proxy_ranges = parse_csv_env_list(&ranges);
        }
        if let Some(rate_limit) = parse_env("KALAMDB_RATE_LIMIT_AUTH_REQUESTS_PER_IP_PER_SEC")? {
            self.rate_limit.max_auth_requests_per_ip_per_sec = rate_limit;
        }
        Ok(())
    }

    fn apply_websocket_env_overrides(&mut self) -> Result<()> {
        if let Some(timeout_secs) = parse_env("KALAMDB_WEBSOCKET_CLIENT_TIMEOUT_SECS")? {
            self.websocket.client_timeout_secs = Some(timeout_secs);
        }
        if let Some(timeout_secs) = parse_env("KALAMDB_WEBSOCKET_AUTH_TIMEOUT_SECS")? {
            self.websocket.auth_timeout_secs = Some(timeout_secs);
        }
        if let Some(interval_secs) = parse_env("KALAMDB_WEBSOCKET_HEARTBEAT_INTERVAL_SECS")? {
            self.websocket.heartbeat_interval_secs = Some(interval_secs);
        }
        Ok(())
    }

    fn apply_storage_env_overrides(&mut self) {
        if let Ok(path) = env::var("KALAMDB_DATA_DIR") {
            self.storage.data_path = path;
        }
    }

    fn apply_cluster_env_overrides(&mut self) -> Result<()> {
        let cluster_id = env::var("KALAMDB_CLUSTER_ID").ok();
        let node_id = env::var("KALAMDB_NODE_ID")
            .ok()
            .or_else(|| env::var("KALAMDB_CLUSTER_NODE_ID").ok());
        let rpc_addr = env::var("KALAMDB_CLUSTER_RPC_ADDR").ok();
        let api_addr = env::var("KALAMDB_CLUSTER_API_ADDR").ok();
        let peers_env = env::var("KALAMDB_CLUSTER_PEERS").ok();

        if cluster_id.is_none()
            && node_id.is_none()
            && rpc_addr.is_none()
            && api_addr.is_none()
            && peers_env.is_none()
        {
            return Ok(());
        }

        let parsed_node_id = match node_id {
            Some(ref value) => value
                .parse::<u64>()
                .map_err(|_| anyhow!("Invalid KALAMDB_NODE_ID value: {value}"))?,
            None => self.cluster.as_ref().map(|cluster| cluster.node_id).ok_or_else(|| {
                anyhow!("KALAMDB_NODE_ID is required when overriding cluster settings")
            })?,
        };

        let cluster = self.cluster.get_or_insert_with(|| ClusterConfig {
            cluster_id: cluster_id.clone().unwrap_or_else(|| "cluster".to_string()),
            node_id: parsed_node_id,
            rpc_addr: rpc_addr.clone().unwrap_or_else(|| "127.0.0.1:2910".to_string()),
            api_addr: api_addr.clone().unwrap_or_else(|| "127.0.0.1:2900".to_string()),
            peers: Vec::new(),
            user_shards: 12,
            shared_shards: 1,
            heartbeat_interval_ms: 250,
            election_timeout_ms: (500, 1000),
            snapshot_policy: "LogsSinceLast(1000)".to_string(),
            max_snapshots_to_keep: 3,
            replication_timeout_ms: 5000,
            reconnect_interval_ms: 3000,
            peer_wait_max_retries: None,
            peer_wait_initial_delay_ms: None,
            peer_wait_max_delay_ms: None,
        });

        if let Some(value) = cluster_id {
            cluster.cluster_id = value;
        }
        cluster.node_id = parsed_node_id;
        if let Some(value) = rpc_addr {
            cluster.rpc_addr = value;
        }
        if let Some(value) = api_addr {
            cluster.api_addr = value;
        }
        if let Some(value) = peers_env {
            cluster.peers = parse_cluster_peers(&value)?;
        }
        Ok(())
    }

    fn apply_rpc_tls_env_overrides(&mut self) {
        if let Some(enabled) = env_truthy("KALAMDB_RPC_TLS_ENABLED") {
            self.rpc_tls.enabled = enabled;
        }
        if let Ok(ca_cert) = env::var("KALAMDB_RPC_TLS_CA_CERT") {
            self.rpc_tls.ca_cert = Some(ca_cert);
        }
        if let Ok(server_cert) = env::var("KALAMDB_RPC_TLS_SERVER_CERT") {
            self.rpc_tls.server_cert = Some(server_cert);
        }
        if let Ok(server_key) = env::var("KALAMDB_RPC_TLS_SERVER_KEY") {
            self.rpc_tls.server_key = Some(server_key);
        }
        if let Some(require_client_cert) = env_truthy("KALAMDB_RPC_TLS_REQUIRE_CLIENT_CERT") {
            self.rpc_tls.require_client_cert = require_client_cert;
        }
    }
}

fn parse_cluster_peers(value: &str) -> Result<Vec<PeerConfig>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut peers = Vec::new();
    for entry in trimmed.split(';') {
        let parts: Vec<&str> = entry.split('@').collect();
        if parts.len() < 3 || parts.len() > 4 {
            return Err(anyhow::anyhow!(
                "Invalid KALAMDB_CLUSTER_PEERS entry '{}'. Expected format: \
                 node_id@rpc_addr@api_addr[@rpc_server_name]",
                entry
            ));
        }

        let node_id = parts[0]
            .trim()
            .parse::<u64>()
            .map_err(|_| anyhow::anyhow!("Invalid peer node_id in '{}': {}", entry, parts[0]))?;

        peers.push(PeerConfig {
            node_id,
            rpc_addr: parts[1].trim().to_string(),
            api_addr: parts[2].trim().to_string(),
            rpc_server_name: parts.get(3).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
        });
    }

    Ok(peers)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;

    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn acquire_env_lock() -> MutexGuard<'static, ()> {
        ENV_MUTEX.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn test_env_override_server_host() {
        let _guard = acquire_env_lock();
        env::set_var("KALAMDB_SERVER_HOST", "0.0.0.0");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.server.host, "0.0.0.0");

        env::remove_var("KALAMDB_SERVER_HOST");
    }

    #[test]
    fn test_env_override_server_port() {
        let _guard = acquire_env_lock();
        env::set_var("KALAMDB_SERVER_PORT", "3900");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.server.port, 3900);

        env::remove_var("KALAMDB_SERVER_PORT");
    }

    #[test]
    fn test_env_override_public_origin_allows_blank_browser_fallback() {
        let _guard = acquire_env_lock();
        env::set_var("KALAMDB_SERVER_PUBLIC_ORIGIN", "");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.server.public_origin.as_deref(), Some(""));
        assert_eq!(config.server.configured_public_origin(), None);

        env::remove_var("KALAMDB_SERVER_PUBLIC_ORIGIN");
    }

    #[test]
    fn test_env_override_log_level() {
        let _guard = acquire_env_lock();
        env::set_var("KALAMDB_LOG_LEVEL", "debug");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.logging.level, "debug");

        env::remove_var("KALAMDB_LOG_LEVEL");
    }

    #[test]
    fn test_env_override_cors_allowed_origins() {
        let _guard = acquire_env_lock();
        env::set_var(
            "KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS",
            "http://localhost:5173, https://admin.example.com",
        );

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(
            config.security.cors.allowed_origins,
            vec![
                "http://localhost:5173".to_string(),
                "https://admin.example.com".to_string()
            ]
        );

        env::remove_var("KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS");
    }

    #[test]
    fn test_env_override_cors_allowed_origins_wildcard() {
        let _guard = acquire_env_lock();
        env::set_var("KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS", "*");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.security.cors.allowed_origins, vec!["*".to_string()]);

        env::remove_var("KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS");
    }

    #[test]
    fn test_env_override_topic_visibility_timeout_secs() {
        let _guard = acquire_env_lock();
        env::remove_var("KALAMDB_VISIBILITY_TIMEOUT_SECS");
        env::set_var("KALAMDB_TOPIC_VISIBILITY_TIMEOUT_SECS", "7");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.topics.visibility_timeout_secs, 7);

        env::remove_var("KALAMDB_TOPIC_VISIBILITY_TIMEOUT_SECS");
    }

    #[test]
    fn test_env_override_topic_visibility_timeout_secs_legacy_alias() {
        let _guard = acquire_env_lock();
        env::remove_var("KALAMDB_TOPIC_VISIBILITY_TIMEOUT_SECS");
        env::set_var("KALAMDB_VISIBILITY_TIMEOUT_SECS", "5");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.topics.visibility_timeout_secs, 5);

        env::remove_var("KALAMDB_VISIBILITY_TIMEOUT_SECS");
    }

    #[test]
    fn test_env_override_topic_retention_settings() {
        let _guard = acquire_env_lock();
        env::set_var("KALAMDB_TOPIC_DEFAULT_RETENTION_SECONDS", "120");
        env::set_var("KALAMDB_TOPIC_DEFAULT_RETENTION_MAX_BYTES", "2048");
        env::set_var("KALAMDB_TOPIC_RETENTION_CHECK_INTERVAL_SECONDS", "30");
        env::set_var("KALAMDB_TOPIC_RETENTION_BATCH_SIZE", "50");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.topics.default_retention_seconds, 120);
        assert_eq!(config.topics.default_retention_max_bytes, 2048);
        assert_eq!(config.topics.retention_check_interval_seconds, 30);
        assert_eq!(config.topics.retention_batch_size, 50);

        env::remove_var("KALAMDB_TOPIC_DEFAULT_RETENTION_SECONDS");
        env::remove_var("KALAMDB_TOPIC_DEFAULT_RETENTION_MAX_BYTES");
        env::remove_var("KALAMDB_TOPIC_RETENTION_CHECK_INTERVAL_SECONDS");
        env::remove_var("KALAMDB_TOPIC_RETENTION_BATCH_SIZE");
    }

    #[test]
    fn test_env_override_trusted_proxy_ranges() {
        let _guard = acquire_env_lock();
        env::set_var("KALAMDB_SECURITY_TRUSTED_PROXY_RANGES", "10.0.1.9,192.168.0.0/24");

        let mut config = ServerConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(
            config.security.trusted_proxy_ranges,
            vec!["10.0.1.9".to_string(), "192.168.0.0/24".to_string()]
        );

        env::remove_var("KALAMDB_SECURITY_TRUSTED_PROXY_RANGES");
    }
}
