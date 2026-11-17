//! A peer speaking another Raft RPC protocol version is rejected before the
//! payload is decoded.

use kalamdb_raft::network::service::{reject_foreign_protocol, RAFT_RPC_PROTOCOL_VERSION};

#[test]
fn foreign_protocol_version_is_rejected_before_payload_decode() {
    let payload = [0xff_u8, 0x00, 0x01];
    let rejected =
        reject_foreign_protocol(RAFT_RPC_PROTOCOL_VERSION + 1).expect_err("foreign version");
    let message = rejected.to_string();
    assert!(
        message.contains("protocol version"),
        "rejection should name the protocol version, got {message}"
    );
    assert!(message.contains("rejected"), "rejection should be explicit, got {message}");
    assert!(
        !message.contains("decode"),
        "the version check must fail before payload decode, got {message}"
    );
    assert_eq!(payload, [0xff, 0x00, 0x01]);

    let older = reject_foreign_protocol(0).expect_err("missing version");
    assert!(older.to_string().contains("protocol version 0"));

    assert!(reject_foreign_protocol(RAFT_RPC_PROTOCOL_VERSION).is_ok());
}
