//! OpenRaft 0.9.25 compatibility fixtures (T091).
//!
//! Loads the bytes written by T090 and decodes them with the current types.
//! After the 0.10 pin change this test is expected to fail until T097.

use std::{fs, path::PathBuf};

use kalamdb_raft::{
    decode_persisted_membership, decode_persisted_state, state_machine::decode, KalamNode,
    RaftCommandBytes,
};
use kalamdb_store::{decode_entity, RaftSnapshotMeta, RaftVote};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openraft-0.9")
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixture_dir().join(name);
    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

#[test]
fn decodes_openraft_09_fixtures() {
    let vote: RaftVote = decode_entity(&read_fixture("vote.bin")).expect("vote");
    assert_eq!(vote.term, 1);
    assert_eq!(vote.voted_for, Some(1));
    assert!(vote.committed);

    let membership =
        decode_persisted_membership(&read_fixture("membership.bin")).expect("membership");
    assert_eq!(membership.log_id().map(|id| id.index), Some(2));
    assert_eq!(membership.log_id().map(|id| id.leader_id.term), Some(1));

    let header = decode_persisted_state(&read_fixture("state_machine_data.bin"))
        .expect("state machine data");
    assert_eq!(header.state_applied_index, 2);
    assert_eq!(header.state_applied_term, 1);
    assert_eq!(header.state, b"fixture-state");

    let snapshot = read_fixture("snapshot.bin");
    let snapshot_header = decode_persisted_state(&snapshot).expect("snapshot file");
    assert_eq!(snapshot_header.state, header.state);

    let meta: RaftSnapshotMeta =
        decode_entity(&read_fixture("raft_snapshot_meta.bin")).expect("snapshot meta");
    assert_eq!(meta.snapshot_id, "1-2-1");
    assert_eq!(meta.last_log_id.map(|id| (id.term, id.index)), Some((1, 2)));
    assert_eq!(meta.size_bytes, snapshot.len() as u64);

    let node: KalamNode = decode(&read_fixture("kalam_node.bin")).expect("node");
    assert_eq!(node.rpc_addr, "127.0.0.1:2910");
    assert_eq!(node.api_addr, "127.0.0.1:2900");

    let record = read_fixture("command_entry.bin");
    assert!(record.len() > 17, "packed log record header plus payload");
    assert_eq!(record[0], 1, "log record version");
}

#[test]
fn raft_command_bytes_display_omits_payload() {
    let secret = b"fixture_ns-secret-payload";
    let bytes = RaftCommandBytes::new(secret.to_vec());
    let shown = bytes.to_string();
    let debug = format!("{bytes:?}");

    assert!(!shown.contains("fixture_ns"), "display leaked payload: {shown}");
    assert!(!shown.contains("secret-payload"), "display leaked payload: {shown}");
    assert!(!debug.contains("fixture_ns"), "debug leaked payload: {debug}");
    assert!(shown.contains("undecoded"), "display: {shown}");
    assert!(shown.contains(&format!("len: {}", secret.len())), "display: {shown}");
}
