use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
};

use kalamdb_configs::PostgresWireSettings;

fn wire_addr_label(settings: &PostgresWireSettings) -> String {
    format!("{}:{}", settings.host, settings.port)
}

fn address_sets_conflict(left: &HashSet<SocketAddr>, right: &HashSet<SocketAddr>) -> bool {
    left.iter().any(|left_addr| {
        right.iter().any(|right_addr| {
            left_addr.port() == right_addr.port()
                && same_ip_family(left_addr.ip(), right_addr.ip())
                && (left_addr.ip() == right_addr.ip()
                    || left_addr.ip().is_unspecified()
                    || right_addr.ip().is_unspecified())
        })
    })
}

fn same_ip_family(left: IpAddr, right: IpAddr) -> bool {
    matches!((left, right), (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)))
}

/// Returns an error message when PostgreSQL wire and HTTP resolve to the same socket.
pub fn http_port_conflict_message(
    settings: &PostgresWireSettings,
    wire_addrs: &HashSet<SocketAddr>,
    http_addrs: &HashSet<SocketAddr>,
    http_addr: &str,
) -> Option<String> {
    if !settings.enabled || !address_sets_conflict(wire_addrs, http_addrs) {
        return None;
    }

    Some(format!(
        "Invalid configuration: HTTP '{http_addr}' and PostgreSQL wire '{}' resolve to at least \
         one identical socket address. Configure distinct ports.",
        wire_addr_label(settings)
    ))
}

/// Returns an error message when PostgreSQL wire and Raft RPC resolve to the same socket.
pub fn rpc_port_conflict_message(
    settings: &PostgresWireSettings,
    wire_addrs: &HashSet<SocketAddr>,
    rpc_addrs: &HashSet<SocketAddr>,
    rpc_addr: &str,
) -> Option<String> {
    if !settings.enabled || !address_sets_conflict(wire_addrs, rpc_addrs) {
        return None;
    }

    Some(format!(
        "Invalid configuration: Raft RPC '{rpc_addr}' and PostgreSQL wire '{}' resolve to at \
         least one identical socket address. Configure distinct ports.",
        wire_addr_label(settings)
    ))
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::*;

    fn localhost_port(port: u16) -> HashSet<SocketAddr> {
        HashSet::from([SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)])
    }

    #[test]
    fn http_port_conflict_detects_shared_bind_address() {
        let settings = PostgresWireSettings {
            enabled: true,
            host: "127.0.0.1".to_string(),
            port: 5432,
            ..PostgresWireSettings::default()
        };
        let shared = localhost_port(5432);

        let message = http_port_conflict_message(&settings, &shared, &shared, "127.0.0.1:2900");
        assert!(message.is_some());
    }

    #[test]
    fn http_port_conflict_ignores_disabled_listener() {
        let settings = PostgresWireSettings::default();
        let shared = localhost_port(5432);

        let message = http_port_conflict_message(&settings, &shared, &shared, "127.0.0.1:2900");
        assert!(message.is_none());
    }

    #[test]
    fn http_port_conflict_detects_wildcard_and_specific_address() {
        let settings = PostgresWireSettings {
            enabled: true,
            host: "0.0.0.0".to_string(),
            port: 5432,
            ..PostgresWireSettings::default()
        };
        let wire_addrs = HashSet::from([SocketAddr::from(([0, 0, 0, 0], 5432))]);
        let http_addrs = localhost_port(5432);

        let message =
            http_port_conflict_message(&settings, &wire_addrs, &http_addrs, "127.0.0.1:5432");

        assert!(message.is_some());
    }

    #[test]
    fn rpc_port_conflict_allows_different_ports() {
        let settings = PostgresWireSettings {
            enabled: true,
            host: "0.0.0.0".to_string(),
            port: 5432,
            ..PostgresWireSettings::default()
        };
        let wire_addrs = HashSet::from([SocketAddr::from(([0, 0, 0, 0], 5432))]);
        let rpc_addrs = localhost_port(5433);

        let message =
            rpc_port_conflict_message(&settings, &wire_addrs, &rpc_addrs, "127.0.0.1:5433");

        assert!(message.is_none());
    }
}
