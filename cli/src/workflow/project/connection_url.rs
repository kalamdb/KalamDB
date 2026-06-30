//! Connection URL validation for workflow init, config, and dev.

use url::Url;

use crate::error::{CLIError, Result};

const ALLOWED_SCHEMES: [&str; 2] = ["http", "https"];

/// Returns true when `server_url` points at the local machine.
pub fn is_loopback_server_url(server_url: &str) -> bool {
    let Ok(url) = Url::parse(server_url) else {
        return false;
    };

    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Validate an HTTP(S) server URL for workflow configuration.
pub fn validate_http_server_url(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CLIError::ConfigurationError("server URL must not be empty".into()));
    }

    let url = Url::parse(trimmed).map_err(|error| {
        CLIError::ConfigurationError(format!("invalid server URL '{trimmed}': {error}"))
    })?;

    let scheme = url.scheme();
    if !ALLOWED_SCHEMES.contains(&scheme) {
        return Err(CLIError::ConfigurationError(format!(
            "server URL must use http or https, got '{scheme}'"
        )));
    }

    if url.host_str().is_none() {
        return Err(CLIError::ConfigurationError("server URL must include a host".into()));
    }

    url.port_or_known_default().ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "server URL '{trimmed}' must include an explicit or default port"
        ))
    })?;

    Ok(trimmed.to_string())
}

/// Extract the port from a validated HTTP(S) server URL.
pub fn parse_server_port(server_url: &str) -> Result<u16> {
    let validated = validate_http_server_url(server_url)?;
    let url = Url::parse(&validated).map_err(|error| {
        CLIError::ConfigurationError(format!("invalid server URL '{validated}': {error}"))
    })?;
    url.port_or_known_default().ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "server URL '{validated}' must include an explicit port"
        ))
    })
}

/// Compare two HTTP(S) server URLs for credential matching.
pub fn server_urls_match(stored: &str, environment: &str) -> bool {
    normalize_server_url_for_match(stored) == normalize_server_url_for_match(environment)
}

fn normalize_server_url_for_match(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Validate the active dev environment URL, including loopback checks for managed servers.
pub fn validate_dev_environment_url(server_url: &str, auto_start_db: bool) -> Result<()> {
    validate_http_server_url(server_url)?;
    if auto_start_db {
        validate_loopback_server_url(server_url)?;
    }
    Ok(())
}

/// Validate that a server URL is HTTP(S) and targets loopback.
pub fn validate_loopback_server_url(value: &str) -> Result<String> {
    let validated = validate_http_server_url(value)?;
    if !is_loopback_server_url(&validated) {
        return Err(CLIError::ConfigurationError(format!(
            "local KalamDB server URL must target localhost, 127.0.0.1, or ::1 (got '{validated}')"
        )));
    }
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_http_server_url_accepts_localhost() {
        assert_eq!(
            validate_http_server_url("http://localhost:2900").unwrap(),
            "http://localhost:2900"
        );
    }

    #[test]
    fn validate_http_server_url_rejects_non_http_schemes() {
        let err = validate_http_server_url("javascript:alert(1)").unwrap_err().to_string();
        assert!(err.contains("http or https"));
    }

    #[test]
    fn validate_loopback_server_url_rejects_remote_host() {
        let err = validate_loopback_server_url("http://db.example.com:2900")
            .unwrap_err()
            .to_string();
        assert!(err.contains("local KalamDB server URL"));
    }

    #[test]
    fn parse_server_port_reads_explicit_port() {
        assert_eq!(parse_server_port("http://localhost:2900").unwrap(), 2900);
    }

    #[test]
    fn server_urls_match_ignores_trailing_slash_and_case() {
        assert!(server_urls_match("http://localhost:2900", "HTTP://LOCALHOST:2900/"));
        assert!(!server_urls_match("http://localhost:2900", "http://localhost:2933"));
    }

    #[test]
    fn validate_dev_environment_url_requires_loopback_for_managed_server() {
        assert!(validate_dev_environment_url("http://db.example.com:2900", true).is_err());
        assert!(validate_dev_environment_url("http://localhost:2900", true).is_ok());
    }

    #[test]
    fn is_loopback_server_url_accepts_ipv6_loopback() {
        assert!(is_loopback_server_url("http://[::1]:2900"));
    }
}
