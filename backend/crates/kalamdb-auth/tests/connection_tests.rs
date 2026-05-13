// Unit tests for connection detection and localhost validation
// T102 [US5] - Test localhost detection for IPv4, IPv6, Unix socket

use kalamdb_commons::models::ConnectionInfo;

#[test]
fn test_localhost_detection_127_0_0_1() {
    let conn = ConnectionInfo::new(Some("127.0.0.1".to_string()));
    assert!(conn.is_localhost(), "127.0.0.1 should be detected as localhost");
}

#[test]
fn test_localhost_detection_127_0_0_1_with_port() {
    let conn = ConnectionInfo::new(Some("127.0.0.1:2900".to_string()));
    assert!(conn.is_localhost(), "127.0.0.1:2900 should be detected as localhost");
}

#[test]
fn test_localhost_detection_ipv6() {
    let conn = ConnectionInfo::new(Some("::1".to_string()));
    assert!(conn.is_localhost(), "::1 should be detected as localhost");
}

#[test]
fn test_localhost_detection_ipv6_with_brackets() {
    let conn = ConnectionInfo::new(Some("[::1]".to_string()));
    assert!(conn.is_localhost(), "[::1] should be detected as localhost");
}

#[test]
fn test_localhost_detection_ipv6_with_port() {
    let conn = ConnectionInfo::new(Some("[::1]:2900".to_string()));
    assert!(conn.is_localhost(), "[::1]:2900 should be detected as localhost");
}

#[test]
fn test_localhost_detection_unix_socket() {
    // Unix sockets typically don't have a remote address
    let conn = ConnectionInfo::new(None);
    assert!(
        !conn.is_localhost(),
        "Unix socket (None) should not be localhost without explicit handling"
    );

    // Note: If we need to support Unix sockets as localhost, we'd need to add
    // a separate field to ConnectionInfo to track the connection type
}

#[test]
fn test_localhost_detection_localhost_name() {
    let conn = ConnectionInfo::new(Some("localhost".to_string()));
    assert!(conn.is_localhost(), "localhost name should be detected as localhost");
}

#[test]
fn test_localhost_detection_localhost_with_port() {
    let conn = ConnectionInfo::new(Some("localhost:2900".to_string()));
    assert!(conn.is_localhost(), "localhost:2900 should be detected as localhost");
}

#[test]
fn test_remote_ipv4() {
    let conn = ConnectionInfo::new(Some("192.168.1.100".to_string()));
    assert!(!conn.is_localhost(), "Remote IPv4 should not be localhost");
}

#[test]
fn test_remote_ipv6() {
    let conn = ConnectionInfo::new(Some("2001:0db8:85a3::8a2e:0370:7334".to_string()));
    assert!(!conn.is_localhost(), "Remote IPv6 should not be localhost");
}
