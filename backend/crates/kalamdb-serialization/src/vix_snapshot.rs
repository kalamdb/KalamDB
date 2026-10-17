//! Compact binary codec for `.vix` vector index snapshot payloads (`KVIX1` v3).
//!
//! Layout:
//! - magic: `KVIX1` (5 bytes)
//! - version: u8 (`3`)
//! - namespace_id: u32 len + UTF-8 bytes
//! - table_name: u32 len + UTF-8 bytes
//! - dimensions: u32
//! - metric: u8 (`0` = Cosine, `1` = L2, `2` = Dot)
//! - generated_at: i64
//! - last_applied_seq: i64
//! - next_key: u64
//! - entry_count: u32
//!   - repeated entry: `key: u64`, `pk_len: u32`, `pk` UTF-8 bytes
//! - index_blob_len: u32, `index_blob` bytes

use kalamdb_commons::models::{TableId, VectorMetric};

use crate::error::{Result, SerializationError};

const VIX_MAGIC: &[u8; 5] = b"KVIX1";
const VIX_CODEC_VERSION: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VixSnapshotEntry {
    pub key: u64,
    pub pk:  String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VixSnapshotFile {
    pub table_id:         TableId,
    pub dimensions:       u32,
    pub metric:           VectorMetric,
    pub generated_at:     i64,
    pub last_applied_seq: i64,
    pub next_key:         u64,
    pub entries:          Vec<VixSnapshotEntry>,
    pub index_blob:       Vec<u8>,
}

/// Encode a `.vix` vector index snapshot payload.
pub fn encode_vix_snapshot(snapshot: &VixSnapshotFile) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(
        256 + snapshot.entries.len().saturating_mul(24) + snapshot.index_blob.len(),
    );
    output.extend_from_slice(VIX_MAGIC);
    output.push(VIX_CODEC_VERSION);

    write_string(&mut output, snapshot.table_id.namespace_id().as_str())?;
    write_string(&mut output, snapshot.table_id.table_name().as_str())?;
    output.extend_from_slice(&snapshot.dimensions.to_le_bytes());
    output.push(encode_metric(snapshot.metric));
    output.extend_from_slice(&snapshot.generated_at.to_le_bytes());
    output.extend_from_slice(&snapshot.last_applied_seq.to_le_bytes());
    output.extend_from_slice(&snapshot.next_key.to_le_bytes());

    let entry_count = u32::try_from(snapshot.entries.len()).map_err(|_| {
        SerializationError::Encode("Too many key entries in vector snapshot".to_string())
    })?;
    output.extend_from_slice(&entry_count.to_le_bytes());
    for entry in &snapshot.entries {
        output.extend_from_slice(&entry.key.to_le_bytes());
        write_string(&mut output, &entry.pk)?;
    }

    let blob_len = u32::try_from(snapshot.index_blob.len()).map_err(|_| {
        SerializationError::Encode(
            "Index blob too large to encode in vector snapshot".to_string(),
        )
    })?;
    output.extend_from_slice(&blob_len.to_le_bytes());
    output.extend_from_slice(&snapshot.index_blob);

    Ok(output)
}

/// Decode a `.vix` vector index snapshot payload.
pub fn decode_vix_snapshot(input: &[u8]) -> Result<VixSnapshotFile> {
    let mut cursor = 0usize;

    let magic = read_bytes(input, &mut cursor, VIX_MAGIC.len())?;
    if magic != VIX_MAGIC {
        return Err(SerializationError::InvalidMagic);
    }

    let version = read_u8(input, &mut cursor)?;
    if version != VIX_CODEC_VERSION {
        return Err(SerializationError::UnsupportedProtocolVersion {
            found:     u16::from(version),
            supported: u16::from(VIX_CODEC_VERSION),
        });
    }

    let namespace_id = read_str(input, &mut cursor)?;
    let table_name = read_str(input, &mut cursor)?;
    let table_id = TableId::try_from_strings(namespace_id, table_name).map_err(|e| {
        SerializationError::Decode(format!("Invalid table_id in vector snapshot: {e}"))
    })?;
    let dimensions = read_u32(input, &mut cursor)?;
    let metric = decode_metric(read_u8(input, &mut cursor)?)?;
    let generated_at = read_i64(input, &mut cursor)?;
    let last_applied_seq = read_i64(input, &mut cursor)?;
    let next_key = read_u64(input, &mut cursor)?;

    let entries_count = read_u32(input, &mut cursor)? as usize;
    let mut entries = Vec::with_capacity(entries_count.min(65_536));
    for _ in 0..entries_count {
        let key = read_u64(input, &mut cursor)?;
        let pk = read_string(input, &mut cursor)?;
        entries.push(VixSnapshotEntry { key, pk });
    }

    let blob_len = read_u32(input, &mut cursor)? as usize;
    let index_blob = read_bytes(input, &mut cursor, blob_len)?.to_vec();

    if cursor != input.len() {
        return Err(SerializationError::Decode(
            "Vector snapshot has trailing bytes".to_string(),
        ));
    }

    Ok(VixSnapshotFile {
        table_id,
        dimensions,
        metric,
        generated_at,
        last_applied_seq,
        next_key,
        entries,
        index_blob,
    })
}

fn encode_metric(metric: VectorMetric) -> u8 {
    match metric {
        VectorMetric::Cosine => 0,
        VectorMetric::L2 => 1,
        VectorMetric::Dot => 2,
    }
}

fn decode_metric(code: u8) -> Result<VectorMetric> {
    match code {
        0 => Ok(VectorMetric::Cosine),
        1 => Ok(VectorMetric::L2),
        2 => Ok(VectorMetric::Dot),
        _ => Err(SerializationError::Decode(format!(
            "Unknown vector metric code {code}"
        ))),
    }
}

fn write_string(output: &mut Vec<u8>, value: &str) -> Result<()> {
    let len = u32::try_from(value.len()).map_err(|_| {
        SerializationError::Encode("String too large to encode in vector snapshot".to_string())
    })?;
    output.extend_from_slice(&len.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn read_bytes<'a>(input: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8]> {
    let start = *cursor;
    let end = start.checked_add(len).ok_or_else(|| {
        SerializationError::Decode("Vector snapshot cursor overflow".to_string())
    })?;
    let slice = input.get(start..end).ok_or_else(|| {
        SerializationError::Decode("Unexpected EOF while decoding vector snapshot".to_string())
    })?;
    *cursor = end;
    Ok(slice)
}

fn read_u8(input: &[u8], cursor: &mut usize) -> Result<u8> {
    Ok(read_bytes(input, cursor, 1)?[0])
}

fn read_u32(input: &[u8], cursor: &mut usize) -> Result<u32> {
    let bytes = read_bytes(input, cursor, 4)?;
    let array: [u8; 4] = bytes.try_into().map_err(|_| {
        SerializationError::Decode("Failed to decode u32 from vector snapshot".to_string())
    })?;
    Ok(u32::from_le_bytes(array))
}

fn read_u64(input: &[u8], cursor: &mut usize) -> Result<u64> {
    let bytes = read_bytes(input, cursor, 8)?;
    let array: [u8; 8] = bytes.try_into().map_err(|_| {
        SerializationError::Decode("Failed to decode u64 from vector snapshot".to_string())
    })?;
    Ok(u64::from_le_bytes(array))
}

fn read_i64(input: &[u8], cursor: &mut usize) -> Result<i64> {
    let bytes = read_bytes(input, cursor, 8)?;
    let array: [u8; 8] = bytes.try_into().map_err(|_| {
        SerializationError::Decode("Failed to decode i64 from vector snapshot".to_string())
    })?;
    Ok(i64::from_le_bytes(array))
}

fn read_str<'a>(input: &'a [u8], cursor: &mut usize) -> Result<&'a str> {
    let len = read_u32(input, cursor)? as usize;
    let bytes = read_bytes(input, cursor, len)?;
    std::str::from_utf8(bytes)
        .map_err(|e| SerializationError::Decode(format!("Invalid UTF-8 string in vector snapshot: {e}")))
}

fn read_string(input: &[u8], cursor: &mut usize) -> Result<String> {
    read_str(input, cursor).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vix_snapshot_roundtrips_typed_table_id_without_column_name() {
        let snapshot = VixSnapshotFile {
            table_id:         TableId::from_strings("app", "docs"),
            dimensions:       3,
            metric:           VectorMetric::Cosine,
            generated_at:     12_345,
            last_applied_seq: 99,
            next_key:         1000,
            entries:          vec![
                VixSnapshotEntry {
                    key: 1,
                    pk:  "row-a".to_string(),
                },
                VixSnapshotEntry {
                    key: 2,
                    pk:  "row-b".to_string(),
                },
            ],
            index_blob:       vec![10, 20, 30, 40],
        };

        let encoded = encode_vix_snapshot(&snapshot).expect("encode vix snapshot");
        let decoded = decode_vix_snapshot(&encoded).expect("decode vix snapshot");
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.table_id.namespace_id().as_str(), "app");
        assert_eq!(decoded.table_id.table_name().as_str(), "docs");
    }

    #[test]
    fn vix_snapshot_rejects_invalid_magic_and_trailing_bytes() {
        assert_eq!(
            decode_vix_snapshot(b"BAD00"),
            Err(SerializationError::InvalidMagic)
        );

        let snapshot = VixSnapshotFile {
            table_id:         TableId::from_strings("app", "docs"),
            dimensions:       2,
            metric:           VectorMetric::Dot,
            generated_at:     1,
            last_applied_seq: 2,
            next_key:         3,
            entries:          Vec::new(),
            index_blob:       vec![1],
        };
        let mut encoded = encode_vix_snapshot(&snapshot).expect("encode");
        encoded.push(0xFF);
        assert!(matches!(
            decode_vix_snapshot(&encoded),
            Err(SerializationError::Decode(_))
        ));
    }
}
