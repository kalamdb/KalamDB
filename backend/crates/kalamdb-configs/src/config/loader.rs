use std::{fs, net::IpAddr, path::Path};

use super::{trusted_proxies::parse_trusted_proxy_entries, types::ServerConfig};
use crate::file_helpers::normalize_dir_path;

fn is_localhost_host(host: &str) -> bool {
    let trimmed = host.trim().trim_matches('[').trim_matches(']');
    trimmed.eq_ignore_ascii_case("localhost")
        || trimmed.parse::<IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

fn extract_host_component(value: &str) -> &str {
    let trimmed = value.trim();
    let without_scheme = trimmed.split_once("://").map(|(_, rest)| rest).unwrap_or(trimmed);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);

    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }

    if authority.matches(':').count() == 1 {
        return authority.rsplit_once(':').map(|(host, _)| host).unwrap_or(authority);
    }

    authority
}

fn endpoint_is_localhost(value: &str) -> bool {
    is_localhost_host(extract_host_component(value))
}

fn has_configured_origin_policy(origins: &[String]) -> bool {
    origins.iter().any(|origin| !origin.trim().is_empty())
}

fn uses_wildcard_origin_policy(origins: &[String]) -> bool {
    origins.iter().any(|origin| origin.trim() == "*")
}

fn ensure_non_zero_u16(value: u16, field_name: &str) -> anyhow::Result<()> {
    if value == 0 {
        return Err(anyhow::anyhow!("{field_name} cannot be 0"));
    }
    Ok(())
}

fn ensure_non_zero_usize(value: usize, field_name: &str) -> anyhow::Result<()> {
    if value == 0 {
        return Err(anyhow::anyhow!("{field_name} cannot be 0"));
    }
    Ok(())
}

fn ensure_positive_i64(value: i64, field_name: &str) -> anyhow::Result<()> {
    if value <= 0 {
        return Err(anyhow::anyhow!("{field_name} must be greater than 0"));
    }
    Ok(())
}

fn ensure_valid_choice(value: &str, valid_values: &[&str], field_name: &str) -> anyhow::Result<()> {
    if valid_values.contains(&value) {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "Invalid {field_name} '{value}'. Must be one of: {}",
        valid_values.join(", ")
    ))
}

fn validate_public_origin(config: &ServerConfig) -> anyhow::Result<()> {
    if let Some(public_origin) = config
        .server
        .public_origin
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let valid_scheme =
            public_origin.starts_with("http://") || public_origin.starts_with("https://");
        if !valid_scheme {
            return Err(anyhow::anyhow!(
                "server.public_origin must start with http:// or https://"
            ));
        }
    }

    Ok(())
}

fn validate_logging(config: &ServerConfig) -> anyhow::Result<()> {
    const VALID_LEVELS: &[&str] = &["error", "warn", "info", "debug", "trace"];
    const VALID_FORMATS: &[&str] = &["compact", "pretty", "json"];

    ensure_valid_choice(&config.logging.level, VALID_LEVELS, "log level")?;
    ensure_valid_choice(&config.logging.format, VALID_FORMATS, "log format")?;

    for (target, level) in &config.logging.targets {
        if !VALID_LEVELS.contains(&level.as_str()) {
            return Err(anyhow::anyhow!(
                "Invalid log level '{}' for target '{}'. Must be one of: {}",
                level,
                target,
                VALID_LEVELS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_limits(config: &ServerConfig) -> anyhow::Result<()> {
    ensure_non_zero_usize(config.limits.max_message_size, "max_message_size")?;
    ensure_non_zero_usize(config.limits.max_query_limit, "max_query_limit")?;

    if config.limits.default_query_limit > config.limits.max_query_limit {
        return Err(anyhow::anyhow!(
            "default_query_limit ({}) cannot exceed max_query_limit ({})",
            config.limits.default_query_limit,
            config.limits.max_query_limit
        ));
    }

    Ok(())
}

fn validate_user_management(config: &ServerConfig) -> anyhow::Result<()> {
    if config.user_management.deletion_grace_period_days < 0 {
        return Err(anyhow::anyhow!("deletion_grace_period_days cannot be negative"));
    }

    if config.user_management.cleanup_job_schedule.trim().is_empty() {
        return Err(anyhow::anyhow!("cleanup_job_schedule cannot be empty"));
    }

    Ok(())
}

fn validate_topics(config: &ServerConfig) -> anyhow::Result<()> {
    ensure_positive_i64(
        config.topics.default_retention_seconds,
        "topics.default_retention_seconds",
    )?;
    ensure_positive_i64(
        config.topics.default_retention_max_bytes,
        "topics.default_retention_max_bytes",
    )?;
    ensure_non_zero_usize(config.topics.retention_batch_size, "topics.retention_batch_size")?;
    Ok(())
}

fn validate_security(config: &ServerConfig) -> anyhow::Result<()> {
    parse_trusted_proxy_entries(&config.security.trusted_proxy_ranges).map_err(|error| {
        anyhow::anyhow!("Invalid security.trusted_proxy_ranges configuration: {}", error)
    })?;

    if is_non_local_http_exposure(config)
        && !has_configured_origin_policy(&config.security.cors.allowed_origins)
    {
        return Err(anyhow::anyhow!(
            "Non-localhost HTTP exposure requires security.cors.allowed_origins to be \
             configured (empty is not allowed)"
        ));
    }

    Ok(())
}

fn validate_cluster_tls(config: &ServerConfig) -> anyhow::Result<()> {
    config
        .rpc_tls
        .validate()
        .map_err(|error| anyhow::anyhow!("Invalid rpc_tls configuration: {}", error))?;

    if let Some(cluster) = &config.cluster {
        cluster
            .validate()
            .map_err(|error| anyhow::anyhow!("Invalid cluster configuration: {}", error))?;

        if !config.rpc_tls.enabled
            && !cluster.peers.is_empty()
            && cluster_uses_non_local_networking(cluster)
        {
            return Err(anyhow::anyhow!(
                "Non-localhost multi-node clusters require rpc_tls.enabled = true"
            ));
        }
    }

    Ok(())
}

fn reject_legacy_oauth_config(content: &str) -> anyhow::Result<()> {
    let Ok(value) = toml::from_str::<toml::Value>(content) else {
        return Ok(());
    };

    if value.get("oauth").is_some() {
        return Err(anyhow::anyhow!(
            "legacy [oauth] and [oauth.providers.*] configuration is no longer supported; use [auth.oidc] for the single external OpenID Connect provider and [auth.local] for username/password login"
        ));
    }

    Ok(())
}

fn validate_auth(config: &ServerConfig) -> anyhow::Result<()> {
    let oidc = &config.auth.oidc;
    if !oidc.enabled {
        return Ok(());
    }

    let issuer = oidc.issuer_str().ok_or_else(|| {
        anyhow::anyhow!("auth.oidc.issuer is required when auth.oidc.enabled = true")
    })?;
    if !(issuer.starts_with("http://") || issuer.starts_with("https://")) {
        return Err(anyhow::anyhow!("auth.oidc.issuer must start with http:// or https://"));
    }

    oidc.client_id_str().ok_or_else(|| {
        anyhow::anyhow!("auth.oidc.client_id is required when auth.oidc.enabled = true")
    })?;

    if oidc.scopes.iter().all(|scope| scope.trim() != "openid") {
        return Err(anyhow::anyhow!("auth.oidc.scopes must include the 'openid' scope"));
    }

    if let Some(endpoint) = oidc.device_authorization_endpoint.as_deref() {
        let endpoint = endpoint.trim();
        if !endpoint.is_empty()
            && !(endpoint.starts_with("http://") || endpoint.starts_with("https://"))
        {
            return Err(anyhow::anyhow!(
                "auth.oidc.device_authorization_endpoint must start with http:// or https://"
            ));
        }
    }

    Ok(())
}

fn is_non_local_http_exposure(config: &ServerConfig) -> bool {
    if !is_localhost_host(&config.server.host) {
        return true;
    }

    config
        .server
        .configured_public_origin()
        .is_some_and(|origin| !endpoint_is_localhost(&origin))
}

fn cluster_uses_non_local_networking(cluster: &super::cluster::ClusterConfig) -> bool {
    !endpoint_is_localhost(&cluster.rpc_addr)
        || !endpoint_is_localhost(&cluster.api_addr)
        || cluster.peers.iter().any(|peer| {
            !endpoint_is_localhost(&peer.rpc_addr) || !endpoint_is_localhost(&peer.api_addr)
        })
}

impl ServerConfig {
    /// Load configuration from a TOML file
    ///
    /// Note: Environment overrides are applied separately via `apply_env_overrides()`.
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .map_err(|e| anyhow::anyhow!("Failed to read config file: {}", e))?;

        Self::from_toml_str(&content)
    }

    pub fn from_toml_str(content: &str) -> anyhow::Result<Self> {
        reject_legacy_oauth_config(content)?;

        let mut config: ServerConfig = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse config file: {}", e))?;

        config.finalize()?;

        Ok(config)
    }

    /// Normalize directory-like paths to absolute paths for consistent runtime behavior.
    ///
    /// This keeps semantics the same (relative paths remain relative to current working dir),
    /// but avoids each subsystem re-implementing path handling.
    fn normalize_paths(&mut self) {
        self.storage.data_path = normalize_dir_path(&self.storage.data_path);
        self.logging.logs_path = normalize_dir_path(&self.logging.logs_path);
    }

    /// Normalize local filesystem paths and validate configuration.
    ///
    /// Call this after applying environment overrides.
    pub fn finalize(&mut self) -> anyhow::Result<()> {
        // Normalize local filesystem paths (rocksdb/logs/storage/snapshots)
        self.normalize_paths();

        self.validate()?;

        Ok(())
    }

    /// Returns true when startup should emit a security warning because the
    /// server is reachable from non-localhost clients while CORS is configured
    /// to accept any browser origin.
    pub fn should_warn_on_non_local_http_wildcard_cors(&self) -> bool {
        is_non_local_http_exposure(self)
            && uses_wildcard_origin_policy(&self.security.cors.allowed_origins)
    }

    /// Validate configuration settings
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure_non_zero_u16(self.server.port, "Server port")?;
        validate_public_origin(self)?;
        validate_logging(self)?;
        validate_limits(self)?;
        validate_user_management(self)?;
        validate_topics(self)?;
        validate_auth(self)?;
        validate_security(self)?;
        validate_cluster_tls(self)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::cluster::{ClusterConfig, PeerConfig},
        *,
    };

    fn local_cluster_config() -> ClusterConfig {
        ClusterConfig {
            cluster_id: "test-cluster".to_string(),
            node_id: 1,
            rpc_addr: "127.0.0.1:2910".to_string(),
            api_addr: "127.0.0.1:2900".to_string(),
            peers: vec![PeerConfig {
                node_id: 2,
                rpc_addr: "127.0.0.2:2910".to_string(),
                api_addr: "127.0.0.2:2900".to_string(),
                rpc_server_name: None,
            }],
            user_shards: 8,
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
        }
    }

    #[test]
    fn test_default_config_is_valid() {
        let config = ServerConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_auth_local_and_oidc_parse() {
        let auth: super::super::types::AuthSettings = toml::from_str(
            r#"
            [local]
            enabled = false

            [oidc]
            enabled = true
            display_name = "Dex"
            issuer = "http://127.0.0.1:5556/dex"
            client_id = "kalamdb"
            scopes = ["openid", "email"]
            device_authorization_endpoint = "http://127.0.0.1:5556/dex/device/code"
            broker_device_flow_enabled = true
            auto_provision = true
            default_role = "dba"
            "#,
        )
        .expect("auth settings should parse");

        assert!(!auth.local.enabled);
        assert!(auth.oidc.enabled);
        assert_eq!(auth.oidc.display_name, "Dex");
        assert_eq!(auth.oidc.issuer_str(), Some("http://127.0.0.1:5556/dex"));
        assert_eq!(auth.oidc.client_id_str(), Some("kalamdb"));
        assert!(auth.oidc.broker_device_flow_enabled);

        let mut config = ServerConfig::default();
        config.auth = auth;
        config.validate().expect("unified auth config should validate");
    }

    #[test]
    fn test_auth_oidc_requires_issuer_and_client_id_when_enabled() {
        let mut config = ServerConfig::default();
        config.auth.oidc.enabled = true;

        let err = config.validate().expect_err("missing issuer should be rejected");
        assert!(err.to_string().contains("auth.oidc.issuer"));

        config.auth.oidc.issuer = Some("https://idp.example.com".to_string());
        let err = config.validate().expect_err("missing client id should be rejected");
        assert!(err.to_string().contains("auth.oidc.client_id"));
    }

    #[test]
    fn test_legacy_oauth_config_is_rejected() {
        let mut value = toml::Value::try_from(ServerConfig::default()).unwrap();
        value.as_table_mut().unwrap().insert(
            "oauth".to_string(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "enabled".to_string(),
                toml::Value::Boolean(true),
            )])),
        );
        let content = toml::to_string(&value).unwrap();

        let err = ServerConfig::from_toml_str(&content)
            .expect_err("legacy oauth config should be rejected");
        assert!(err.to_string().contains("legacy [oauth]"));
        assert!(err.to_string().contains("[auth.oidc]"));
    }

    #[test]
    fn test_server_example_config_stays_in_sync() {
        let example = include_str!("../../../../server.example.toml");

        let mut config: ServerConfig = toml::from_str(example).expect("parse server.example.toml");
        config.finalize().expect("validate server.example.toml");

        for required_snippet in [
            "transaction_timeout_secs",
            "max_transaction_buffer_bytes",
            "[user_management]",
            "[files]",
            "[topics]",
            "visibility_timeout_secs",
            "default_retention_seconds",
            "default_retention_max_bytes",
            "retention_check_interval_seconds",
            "retention_batch_size",
        ] {
            assert!(
                example.contains(required_snippet),
                "server.example.toml should include {required_snippet}"
            );
        }

        for stale_snippet in [
            "disable_common_password_check",
            "auto_create_users_from_provider",
            "min_replication_nodes",
            "snapshot_threshold",
        ] {
            assert!(
                !example.contains(stale_snippet),
                "server.example.toml should not mention stale key {stale_snippet}"
            );
        }
    }

    #[test]
    fn test_invalid_port() {
        let mut config = ServerConfig::default();
        config.server.port = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_effective_public_origin_uses_localhost_fallback() {
        let mut config = ServerConfig::default();
        config.server.port = 3900;

        assert_eq!(config.server.effective_public_origin(), "http://localhost:3900");

        config.server.public_origin = Some("https://db.example.com/".to_string());

        assert_eq!(config.server.effective_public_origin(), "https://db.example.com");
    }

    #[test]
    fn test_configured_public_origin_treats_blank_as_unset() {
        let mut config = ServerConfig::default();

        config.server.public_origin = Some("   ".to_string());

        assert_eq!(config.server.configured_public_origin(), None);

        config.server.public_origin = Some("https://db.example.com/".to_string());

        assert_eq!(
            config.server.configured_public_origin().as_deref(),
            Some("https://db.example.com")
        );
    }

    #[test]
    fn test_invalid_log_level() {
        let mut config = ServerConfig::default();
        config.logging.level = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_trusted_proxy_ranges() {
        let mut config = ServerConfig::default();
        config.security.trusted_proxy_ranges = vec!["nope".to_string()];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_public_origin_scheme() {
        let mut config = ServerConfig::default();
        config.server.public_origin = Some("db.example.com".to_string());

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_non_localhost_bind_requires_configured_browser_origins() {
        let mut config = ServerConfig::default();
        config.server.host = "0.0.0.0".to_string();

        let err = config
            .validate()
            .expect_err("non-localhost bind should require configured origins");
        assert!(err.to_string().contains("configured"));
    }

    #[test]
    fn test_non_localhost_bind_allows_wildcard_browser_origins_with_warning() {
        let mut config = ServerConfig::default();
        config.server.host = "0.0.0.0".to_string();
        config.security.cors.allowed_origins = vec!["*".to_string()];

        assert!(config.validate().is_ok());
        assert!(config.should_warn_on_non_local_http_wildcard_cors());
    }

    #[test]
    fn test_non_localhost_bind_allows_explicit_browser_origins() {
        let mut config = ServerConfig::default();
        config.server.host = "0.0.0.0".to_string();
        config.security.cors.allowed_origins = vec![
            "http://localhost:2900".to_string(),
            "http://127.0.0.1:2900".to_string(),
        ];

        assert!(config.validate().is_ok());
        assert!(!config.should_warn_on_non_local_http_wildcard_cors());
    }

    #[test]
    fn test_localhost_bind_does_not_warn_on_wildcard_browser_origins() {
        let mut config = ServerConfig::default();
        config.security.cors.allowed_origins = vec!["*".to_string()];

        assert!(config.validate().is_ok());
        assert!(!config.should_warn_on_non_local_http_wildcard_cors());
    }

    #[test]
    fn test_external_cluster_requires_rpc_tls() {
        let mut config = ServerConfig::default();
        config.cluster = Some(ClusterConfig {
            rpc_addr: "10.0.0.1:2910".to_string(),
            api_addr: "http://10.0.0.1:2900".to_string(),
            peers: vec![PeerConfig {
                node_id: 2,
                rpc_addr: "10.0.0.2:2910".to_string(),
                api_addr: "http://10.0.0.2:2900".to_string(),
                rpc_server_name: None,
            }],
            ..local_cluster_config()
        });

        let err = config.validate().expect_err("external cluster should require rpc_tls");
        assert!(err.to_string().contains("rpc_tls.enabled = true"));
    }

    #[test]
    fn test_local_cluster_allows_disabled_rpc_tls() {
        let mut config = ServerConfig::default();
        config.cluster = Some(local_cluster_config());

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_enabled_rpc_tls_requires_material() {
        let mut config = ServerConfig::default();
        config.rpc_tls.enabled = true;

        let err = config.validate().expect_err("enabled rpc_tls without certs must be rejected");
        assert!(err.to_string().contains("ca_cert"));
    }
}
