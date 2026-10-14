pub(crate) use kalamdb_serialization::{VixSnapshotEntry, VixSnapshotFile};

pub(crate) fn encode_snapshot(snapshot: &VixSnapshotFile) -> Result<Vec<u8>, String> {
    kalamdb_serialization::encode_vix_snapshot(snapshot).map_err(|e| e.to_string())
}

pub(crate) fn decode_snapshot(input: &[u8]) -> Result<VixSnapshotFile, String> {
    kalamdb_serialization::decode_vix_snapshot(input).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::TableId;
    use kalamdb_system::VectorMetric;

    use super::*;

    #[test]
    fn test_snapshot_codec_roundtrip() {
        let snapshot = VixSnapshotFile {
            table_id:         TableId::from_strings("app", "docs"),
            dimensions:       3,
            metric:           VectorMetric::Cosine,
            generated_at:     12345,
            last_applied_seq: 99,
            next_key:         1000,
            entries:          vec![
                VixSnapshotEntry {
                    key: 1,
                    pk:  "a".to_string(),
                },
                VixSnapshotEntry {
                    key: 2,
                    pk:  "b".to_string(),
                },
            ],
            index_blob:       vec![10, 20, 30, 40],
        };

        let encoded = encode_snapshot(&snapshot).expect("encode");
        let decoded = decode_snapshot(&encoded).expect("decode");
        assert_eq!(decoded, snapshot);
    }
}
