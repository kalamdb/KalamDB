//! Version-merge contract: hot/cold MVCC selects by a single [`VersionId`].
//! Snapshot bounds apply before winner selection. Equal versions are one row.

use kalamdb_commons::ids::{RaftVersionId, VersionId};
use kalamdb_datafusion_sources::exec::{
    select_latest_versions, version_ordering, PkBucketKey, SelectedVersion, VersionCandidate,
};

fn v(raw: i64) -> VersionId {
    VersionId::try_from_i64(raw).unwrap()
}

fn raft(log_index: u64, ordinal: u32) -> VersionId {
    RaftVersionId::try_new(log_index, ordinal).unwrap().version()
}

fn snapshot_through_log(log_index: u64) -> VersionId {
    // Inclusive upper bound covering every ordinal of `log_index`.
    VersionId::try_from_raw((log_index << 16) | u64::from(u16::MAX)).unwrap()
}

#[test]
fn greater_version_wins() {
    use std::cmp::Ordering::*;
    assert_eq!(version_ordering(v(10), v(9)), Greater);
    assert_eq!(version_ordering(v(5), v(6)), Less);
    assert_eq!(version_ordering(v(5), v(5)), Equal);
}

#[test]
fn newer_hot_beats_older_cold() {
    let winners = select_latest_versions(
        vec![VersionCandidate::new(PkBucketKey::Int(1), v(2), false, "hot")],
        vec![VersionCandidate::new(PkBucketKey::Int(1), v(1), false, "cold")],
        None,
        false,
    );

    assert_eq!(winners.len(), 1);
    match &winners[0] {
        SelectedVersion::Hot(payload) => assert_eq!(*payload, "hot"),
        SelectedVersion::Cold(payload) => panic!("expected hot winner, got {payload}"),
    }
}

#[test]
fn newer_cold_beats_older_hot() {
    let winners = select_latest_versions(
        vec![VersionCandidate::new("a".to_string(), v(1), false, "hot-a")],
        vec![VersionCandidate::new("a".to_string(), v(2), false, "cold-a")],
        None,
        false,
    );

    assert_eq!(winners.len(), 1);
    match &winners[0] {
        SelectedVersion::Cold(payload) => assert_eq!(*payload, "cold-a"),
        SelectedVersion::Hot(payload) => panic!("expected cold winner, got {payload}"),
    }
}

#[test]
fn equal_version_is_one_row() {
    let winners = select_latest_versions(
        vec![VersionCandidate::new(PkBucketKey::Int(1), v(7), false, "hot")],
        vec![VersionCandidate::new(PkBucketKey::Int(1), v(7), false, "cold")],
        None,
        false,
    );

    assert_eq!(winners.len(), 1);
    match &winners[0] {
        SelectedVersion::Hot(payload) => assert_eq!(*payload, "hot"),
        SelectedVersion::Cold(payload) => panic!("equal versions must collapse to one row, got {payload}"),
    }
}

#[test]
fn snapshot_hides_newer_tier_so_older_visible_wins() {
    // Snapshot excludes log-index 501; visible 490 must beat the hidden newer row.
    let winners = select_latest_versions(
        vec![VersionCandidate::new(
            "a".to_string(),
            raft(501, 0),
            false,
            "hot-hidden",
        )],
        vec![VersionCandidate::new(
            "a".to_string(),
            raft(490, 0),
            false,
            "cold-visible",
        )],
        Some(snapshot_through_log(500)),
        false,
    );

    assert_eq!(winners.len(), 1);
    match &winners[0] {
        SelectedVersion::Cold(payload) => assert_eq!(*payload, "cold-visible"),
        SelectedVersion::Hot(payload) => panic!("snapshot must hide 501, got {payload}"),
    }
}

#[test]
fn tombstone_of_winner_hides_row_unless_keep_deleted() {
    let winners = select_latest_versions(
        vec![VersionCandidate::new("b".to_string(), v(5), false, "hot-b")],
        vec![VersionCandidate::new("b".to_string(), v(6), true, "cold-b-delete")],
        None,
        false,
    );
    assert!(winners.is_empty());

    let kept = select_latest_versions(
        vec![VersionCandidate::new("b".to_string(), v(5), false, "hot-b")],
        vec![VersionCandidate::new("b".to_string(), v(6), true, "cold-b-delete")],
        None,
        true,
    );
    assert_eq!(kept.len(), 1);
}

#[test]
fn version_fallback_does_not_collide_with_text_version_key() {
    let winners = select_latest_versions(
        vec![VersionCandidate::new(
            PkBucketKey::Version(1),
            v(1),
            false,
            "version",
        )],
        vec![VersionCandidate::new(
            PkBucketKey::Text("_version:1".to_string()),
            v(2),
            false,
            "text",
        )],
        None,
        false,
    );

    assert_eq!(winners.len(), 2);
}

#[test]
fn integer_bucket_key_does_not_equal_stringified_integer() {
    assert_ne!(PkBucketKey::Int(1), PkBucketKey::Text("1".to_string()));
    assert_ne!(
        PkBucketKey::Version(1),
        PkBucketKey::Text("_version:1".to_string())
    );
}
