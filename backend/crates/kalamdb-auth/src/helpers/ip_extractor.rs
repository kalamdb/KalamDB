//! Secure client IP extraction with anti-spoofing protection
//!
//! This module provides secure extraction of client IP addresses from HTTP requests,
//! with protection against header spoofing attacks that attempt to bypass localhost checks.

use std::{
    net::IpAddr,
    sync::{OnceLock, RwLock},
};

use actix_web::{
    http::header::{HeaderMap, HeaderValue},
    HttpRequest,
};
use ipnet::IpNet;
use kalamdb_commons::models::ConnectionInfo;
use log::warn;
use once_cell::sync::Lazy;

static TRUSTED_PROXY_RANGES: Lazy<RwLock<Vec<IpNet>>> = Lazy::new(|| RwLock::new(Vec::new()));

/// Extract client IP address with security checks against header spoofing
///
/// # Security Model
///
/// 1. **X-Forwarded-For Validation**: Rejects localhost values in X-Forwarded-For header
///    - Prevents attackers from spoofing `X-Forwarded-For: 127.0.0.1` to bypass security
///    - If localhost is detected in header, falls back to peer_addr (real IP)
///
/// 2. **Trust Hierarchy**:
///    - First: X-Forwarded-For (if present and not localhost)
///    - Fallback: peer_addr (direct connection IP)
///
/// 3. **Attack Prevention**:
///    - Blocks: `127.0.0.1`, `::1`, `127.*`, `localhost` in X-Forwarded-For
///    - Logs warning when spoofing attempt is detected
///
/// # Examples
///
/// ```rust
/// use actix_web::HttpRequest;
/// use kalamdb_auth::extract_client_ip_secure;
///
/// fn handler(req: HttpRequest) {
///     let client_ip = extract_client_ip_secure(&req);
///     println!("Client IP: {:?}", client_ip);
/// }
/// ```
pub fn init_trusted_proxy_ranges(entries: &[String]) -> anyhow::Result<()> {
    let parsed = kalamdb_configs::parse_trusted_proxy_entries(entries)?;
    *TRUSTED_PROXY_RANGES.write().unwrap_or_else(std::sync::PoisonError::into_inner) = parsed;
    Ok(())
}

/// Check direct localhost access to administrative endpoints.
///
/// Never use a forwarded address to grant local access. A loopback TCP peer
/// may itself be a public reverse proxy. Requests carrying proxy headers are
/// denied, even when the forwarded address claims to be localhost.
///
/// If a proxy strips *all* forwarding headers it cannot be distinguished from
/// a direct local client. Public proxies MUST block these routes themselves.
pub fn is_direct_localhost_request(req: &HttpRequest) -> bool {
    let Some(peer) = req.peer_addr() else {
        return false;
    };
    if !peer.ip().is_loopback() {
        return false;
    }
    const PROXY_HEADERS: &[&str] = &[
        "forwarded",
        "x-forwarded-for",
        "x-real-ip",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-port",
        "x-original-forwarded-for",
        "x-envoy-external-address",
        "cf-connecting-ip",
        "true-client-ip",
    ];
    !PROXY_HEADERS.iter().any(|name| req.headers().contains_key(*name))
}

pub fn extract_client_ip_addr_secure(
    peer_addr: Option<IpAddr>,
    headers: &HeaderMap,
) -> Option<IpAddr> {
    // No proxy headers means the peer address is the client. Skip the trusted
    // range lock on that path.
    if !headers.contains_key("x-forwarded-for") && !headers.contains_key("x-real-ip") {
        return peer_addr;
    }

    let trusted_proxy_ranges =
        TRUSTED_PROXY_RANGES.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    extract_client_ip_addr_with_trusted_ranges(peer_addr, headers, &trusted_proxy_ranges)
}

pub fn extract_client_ip_secure(req: &HttpRequest) -> ConnectionInfo {
    let peer = req.peer_addr().map(|addr| addr.ip());
    let headers = req.headers();
    if !headers.contains_key("x-forwarded-for") && !headers.contains_key("x-real-ip") {
        return connection_info_for_peer(peer);
    }
    extract_client_ip_addr_secure(peer, headers)
        .map(|ip| connection_info_for_peer(Some(ip)))
        .unwrap_or_else(|| ConnectionInfo::new(None))
}

fn connection_info_for_peer(peer: Option<IpAddr>) -> ConnectionInfo {
    match peer {
        Some(IpAddr::V4(ip)) if ip.octets() == [127, 0, 0, 1] => loopback_v4(),
        Some(IpAddr::V6(ip)) if ip.is_loopback() => loopback_v6(),
        Some(ip) => ConnectionInfo::new(Some(ip.to_string())),
        None => ConnectionInfo::new(None),
    }
}

fn loopback_v4() -> ConnectionInfo {
    static LOOPBACK: OnceLock<ConnectionInfo> = OnceLock::new();
    LOOPBACK
        .get_or_init(|| ConnectionInfo::new(Some("127.0.0.1".to_string())))
        .clone()
}

fn loopback_v6() -> ConnectionInfo {
    static LOOPBACK: OnceLock<ConnectionInfo> = OnceLock::new();
    LOOPBACK.get_or_init(|| ConnectionInfo::new(Some("::1".to_string()))).clone()
}

fn extract_client_ip_addr_with_trusted_ranges(
    peer_addr: Option<IpAddr>,
    headers: &HeaderMap,
    trusted_proxy_ranges: &[IpNet],
) -> Option<IpAddr> {
    if peer_addr.is_some_and(|ip| is_trusted_proxy_peer(ip, trusted_proxy_ranges)) {
        if let Some(ip) =
            extract_proxy_header_ip(headers.get("X-Forwarded-For"), true, "X-Forwarded-For")
        {
            return Some(ip);
        }

        if let Some(ip) = extract_proxy_header_ip(headers.get("X-Real-IP"), false, "X-Real-IP") {
            return Some(ip);
        }
    } else if headers.contains_key("X-Forwarded-For") || headers.contains_key("X-Real-IP") {
        warn!("Security: Ignoring proxy headers from untrusted peer {:?}", peer_addr);
    }

    peer_addr
}

fn is_trusted_proxy_peer(peer_addr: IpAddr, trusted_proxy_ranges: &[IpNet]) -> bool {
    peer_addr.is_loopback() || trusted_proxy_ranges.iter().any(|range| range.contains(&peer_addr))
}

fn extract_proxy_header_ip(
    header: Option<&HeaderValue>,
    first_csv_value: bool,
    header_name: &str,
) -> Option<IpAddr> {
    let header_value = header?.to_str().ok()?;
    let candidate = if first_csv_value {
        header_value.split(',').next().unwrap_or("").trim()
    } else {
        header_value.trim()
    };

    if candidate.is_empty() {
        return None;
    }

    if is_localhost_address(candidate) {
        warn!(
            "Security: Rejected localhost value in trusted {} header: '{}'. Using peer_addr \
             instead.",
            header_name, candidate
        );
        return None;
    }

    candidate.parse::<IpAddr>().ok()
}

/// Check if an IP address string represents localhost
///
/// Detects all common localhost representations:
/// - IPv4: 127.0.0.1, 127.*, 127.x.x.x
/// - IPv6: ::1
/// - Hostname: localhost
///
/// # Examples
///
/// ```rust
/// use kalamdb_auth::is_localhost_address;
///
/// assert!(is_localhost_address("127.0.0.1"));
/// assert!(is_localhost_address("127.5.8.3"));
/// assert!(is_localhost_address("::1"));
/// assert!(is_localhost_address("localhost"));
/// assert!(!is_localhost_address("192.168.1.1"));
/// ```
pub fn is_localhost_address(ip: &str) -> bool {
    ip == "127.0.0.1"
        || ip == "::1"
        || ip.starts_with("127.")
        || ip.eq_ignore_ascii_case("localhost")
}

#[cfg(test)]
mod tests {
    use actix_web::http::header::{HeaderMap, HeaderName, HeaderValue};

    use super::*;

    #[test]
    fn test_is_localhost_address() {
        // IPv4 localhost
        assert!(is_localhost_address("127.0.0.1"));
        assert!(is_localhost_address("127.1.2.3"));
        assert!(is_localhost_address("127.255.255.255"));

        // IPv6 localhost
        assert!(is_localhost_address("::1"));

        // Hostname
        assert!(is_localhost_address("localhost"));
        assert!(is_localhost_address("LOCALHOST"));
        assert!(is_localhost_address("LocalHost"));

        // Non-localhost addresses
        assert!(!is_localhost_address("192.168.1.1"));
        assert!(!is_localhost_address("10.0.0.1"));
        assert!(!is_localhost_address("8.8.8.8"));
        assert!(!is_localhost_address("::2"));
        assert!(!is_localhost_address(""));
    }

    #[test]
    fn test_localhost_bypass_attempt() {
        // These should all be detected as localhost (bypass attempts)
        let bypass_attempts = vec![
            "127.0.0.1",
            "127.1.1.1",
            "127.255.255.254",
            "::1",
            "localhost",
            "LOCALHOST",
        ];

        for attempt in bypass_attempts {
            assert!(
                is_localhost_address(attempt),
                "Failed to detect localhost bypass attempt: {}",
                attempt
            );
        }
    }

    #[test]
    fn test_loopback_proxy_headers_are_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("203.0.113.8, 10.0.0.1"),
        );

        let ip = extract_client_ip_addr_with_trusted_ranges(
            Some("127.0.0.1".parse().unwrap()),
            &headers,
            &[],
        );

        assert_eq!(ip, Some("203.0.113.8".parse().unwrap()));
    }

    #[test]
    fn test_trusted_proxy_range_allows_forwarded_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("203.0.113.8"),
        );
        let trusted_ranges =
            kalamdb_configs::parse_trusted_proxy_entries(&["10.0.0.0/8".to_string()]).unwrap();

        let ip = extract_client_ip_addr_with_trusted_ranges(
            Some("10.0.1.9".parse().unwrap()),
            &headers,
            &trusted_ranges,
        );

        assert_eq!(ip, Some("203.0.113.8".parse().unwrap()));
    }

    #[test]
    fn test_untrusted_proxy_headers_are_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("203.0.113.8"),
        );

        let ip = extract_client_ip_addr_with_trusted_ranges(
            Some("10.0.1.9".parse().unwrap()),
            &headers,
            &[],
        );

        assert_eq!(ip, Some("10.0.1.9".parse().unwrap()));
    }

    #[test]
    fn test_localhost_spoofing_is_rejected_even_for_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("127.0.0.1"),
        );
        let trusted_ranges =
            kalamdb_configs::parse_trusted_proxy_entries(&["10.0.0.0/8".to_string()]).unwrap();

        let ip = extract_client_ip_addr_with_trusted_ranges(
            Some("10.0.1.9".parse().unwrap()),
            &headers,
            &trusted_ranges,
        );

        assert_eq!(ip, Some("10.0.1.9".parse().unwrap()));
    }
    #[test]
    fn direct_loopback_without_forwarding_headers_is_local() {
        for addr in ["127.0.0.1:2900", "[::1]:2900"] {
            let req = actix_web::test::TestRequest::default()
                .peer_addr(addr.parse().unwrap())
                .to_http_request();
            assert!(is_direct_localhost_request(&req), "{addr}");
        }
    }

    #[test]
    fn localhost_proxy_headers_and_remote_peers_are_never_local() {
        let unknown = actix_web::test::TestRequest::default().to_http_request();
        assert!(!is_direct_localhost_request(&unknown));

        let remote = actix_web::test::TestRequest::default()
            .peer_addr("203.0.113.8:2900".parse().unwrap())
            .to_http_request();
        assert!(!is_direct_localhost_request(&remote));

        for header in [
            "Forwarded",
            "X-Forwarded-For",
            "X-Real-IP",
            "X-Forwarded-Host",
            "X-Forwarded-Proto",
            "X-Forwarded-Port",
            "X-Original-Forwarded-For",
            "X-Envoy-External-Address",
            "CF-Connecting-IP",
            "True-Client-IP",
        ] {
            let req = actix_web::test::TestRequest::default()
                .peer_addr("127.0.0.1:2900".parse().unwrap())
                .insert_header((header, "198.51.100.8"))
                .to_http_request();
            assert!(!is_direct_localhost_request(&req), "{header}");
        }
    }

}
