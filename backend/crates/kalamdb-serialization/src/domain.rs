//! Lean shared domain-value and endpoint-framed binary codec (`T069`, `T070`, `T082`).
//!
//! Compiles without Arrow, DataFusion, RocksDB, HTTP servers, or V8 so it can be shared
//! directly across backend crates, `link-common`, browser/Node WASM (`kalam-link-wasm`),
//! and Dart FRB (`kalam-link-dart`).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, SerializationError};

/// Four-byte magic identifying a current-epoch Kalam domain frame (`KDOM`).
pub const DOMAIN_MAGIC: [u8; 4] = *b"KDOM";

/// Current coordinated protocol/format/API epoch (`FR-029`, `FR-061`).
pub const DOMAIN_EPOCH: u16 = 1;

/// Current domain value/framing grammar version.
pub const DOMAIN_CODEC_VERSION: u16 = 1;

/// Fixed 16-byte frame header length:
/// `magic: [u8; 4]` + `epoch: u16 LE` + `codec_version: u16 LE` + `kind: u8` + `flags: u8`
/// + `descriptor_short_id: u16 LE` + `payload_len: u32 LE`.
pub const DOMAIN_FRAME_HEADER_LEN: usize = 16;

/// Default maximum wire bytes per public RPC/WS frame (16 MiB).
pub const DEFAULT_MAX_FRAME_WIRE_BYTES: usize = 16 * 1024 * 1024;
/// Default maximum decoded bytes per domain frame (64 MiB).
pub const DEFAULT_MAX_DOMAIN_DECODED_BYTES: usize = 64 * 1024 * 1024;
/// Default maximum collection elements per domain frame.
pub const DEFAULT_MAX_DOMAIN_ELEMENTS: usize = 1_048_576;
/// Default maximum nesting depth for composite/array values.
pub const DEFAULT_MAX_DOMAIN_DEPTH: usize = 32;
/// Default maximum traversal work units per decode operation.
pub const DEFAULT_MAX_DOMAIN_WORK: usize = 2_097_152;
/// Maximum initial preallocation per untrusted collection header.
const MAX_INITIAL_CAPACITY: usize = 1_024;

// Value tags (explicit little-endian fixed widths and presence states).
const DTAG_OMITTED: u8 = 0x00;
const DTAG_SQL_NULL: u8 = 0x01;
const DTAG_JSON_NULL: u8 = 0x02;
const DTAG_VOID: u8 = 0x03;
const DTAG_BOOL: u8 = 0x04;
const DTAG_I8: u8 = 0x05;
const DTAG_I16: u8 = 0x06;
const DTAG_I32: u8 = 0x07;
const DTAG_I64: u8 = 0x08;
const DTAG_U8: u8 = 0x09;
const DTAG_U16: u8 = 0x0A;
const DTAG_U32: u8 = 0x0B;
const DTAG_U64: u8 = 0x0C;
const DTAG_F32: u8 = 0x0D;
const DTAG_F64: u8 = 0x0E;
const DTAG_DECIMAL128: u8 = 0x0F;
const DTAG_DATE32: u8 = 0x10;
const DTAG_TIME_MICROS: u8 = 0x11;
const DTAG_TIMESTAMP_MICROS: u8 = 0x12;
const DTAG_TIMESTAMPTZ_MICROS: u8 = 0x13;
const DTAG_UUID: u8 = 0x14;
const DTAG_UTF8: u8 = 0x15;
const DTAG_BYTES: u8 = 0x16;
const DTAG_JSON: u8 = 0x17;
const DTAG_FILE_REF: u8 = 0x18;
const DTAG_ENUM: u8 = 0x19;
pub(crate) const DTAG_COMPOSITE: u8 = 0x1A;
const DTAG_ARRAY: u8 = 0x1B;
const DTAG_ROW_SET: u8 = 0x1C;

/// Endpoint trust surfaces for message-kind allowlisting (`FR-051`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EndpointSurface {
    PublicRpc,
    WebSocket,
    InternalPersistence,
    InternalRaft,
}

/// Distinct wire/storage message kinds sharing the domain value codec (`FR-051`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DomainMessageKind {
    // Public HTTP/native RPC kinds (0x10..=0x1F)
    RpcRequest = 0x10,
    RpcResult = 0x11,
    RpcError = 0x12,

    // WebSocket live/control kinds (0x20..=0x2F)
    WsAuth = 0x20,
    WsSubscribe = 0x21,
    WsUnsubscribe = 0x22,
    WsPing = 0x23,
    WsSnapshotBegin = 0x24,
    WsSnapshotBatch = 0x25,
    WsSnapshotEnd = 0x26,
    WsChanges = 0x27,
    WsAck = 0x28,
    WsCursor = 0x29,
    WsReset = 0x2A,
    WsError = 0x2B,

    // Internal durable storage kinds (0x40..=0x4F)
    DurableRow = 0x40,
    DurableCatalog = 0x41,
    DurableStream = 0x42,
    DurableReceipt = 0x43,

    // Internal Raft consensus kinds (0x50..=0x5F)
    RaftCommand = 0x50,
    RaftSnapshot = 0x51,
}

impl DomainMessageKind {
    pub fn from_u8(tag: u8) -> Result<Self> {
        match tag {
            0x10 => Ok(Self::RpcRequest),
            0x11 => Ok(Self::RpcResult),
            0x12 => Ok(Self::RpcError),
            0x20 => Ok(Self::WsAuth),
            0x21 => Ok(Self::WsSubscribe),
            0x22 => Ok(Self::WsUnsubscribe),
            0x23 => Ok(Self::WsPing),
            0x24 => Ok(Self::WsSnapshotBegin),
            0x25 => Ok(Self::WsSnapshotBatch),
            0x26 => Ok(Self::WsSnapshotEnd),
            0x27 => Ok(Self::WsChanges),
            0x28 => Ok(Self::WsAck),
            0x29 => Ok(Self::WsCursor),
            0x2A => Ok(Self::WsReset),
            0x2B => Ok(Self::WsError),
            0x40 => Ok(Self::DurableRow),
            0x41 => Ok(Self::DurableCatalog),
            0x42 => Ok(Self::DurableStream),
            0x43 => Ok(Self::DurableReceipt),
            0x50 => Ok(Self::RaftCommand),
            0x51 => Ok(Self::RaftSnapshot),
            other => Err(SerializationError::Decode(format!(
                "unknown domain message kind 0x{other:02x}"
            ))),
        }
    }

    /// Enforce endpoint allowlists so valid internal Raft/persistence frames cannot be
    /// accepted on public HTTP RPC or WebSocket endpoints (`FR-051`).
    #[inline]
    pub fn is_allowed_on(self, surface: EndpointSurface) -> bool {
        match surface {
            EndpointSurface::PublicRpc => {
                matches!(self, Self::RpcRequest | Self::RpcResult | Self::RpcError)
            },
            EndpointSurface::WebSocket => matches!(
                self,
                Self::WsAuth
                    | Self::WsSubscribe
                    | Self::WsUnsubscribe
                    | Self::WsPing
                    | Self::WsSnapshotBegin
                    | Self::WsSnapshotBatch
                    | Self::WsSnapshotEnd
                    | Self::WsChanges
                    | Self::WsAck
                    | Self::WsCursor
                    | Self::WsReset
                    | Self::WsError
            ),
            EndpointSurface::InternalPersistence => matches!(
                self,
                Self::DurableRow
                    | Self::DurableCatalog
                    | Self::DurableStream
                    | Self::DurableReceipt
            ),
            EndpointSurface::InternalRaft => {
                matches!(self, Self::RaftCommand | Self::RaftSnapshot)
            },
        }
    }
}

/// Typed stored file reference in domain payloads (`FR-005`, `FR-040`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainFileRef {
    pub id:     String,
    pub sub:    String,
    pub name:   String,
    pub size:   i64,
    pub mime:   String,
    pub sha256: String,
    pub shard:  Option<i32>,
}

/// Canonical domain value model shared across backend, `link-common`, WASM, and FRB (`FR-040`,
/// `FR-048`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DomainValue {
    /// Field was omitted from input/patch; distinct from `SqlNull` and `JsonNull`.
    Omitted,
    /// SQL `NULL` value.
    SqlNull,
    /// JSON `null` value inside a `JSON`/`JSONB` column or parameter.
    JsonNull,
    /// `VOID` result from a procedure or void-returning function.
    Void,
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    Decimal128 {
        unscaled:  i128,
        precision: u8,
        scale:     i8,
    },
    Date32(i32),
    TimeMicros(i64),
    TimestampMicros(i64),
    TimestampTzMicros(i64),
    Uuid([u8; 16]),
    Utf8(String),
    Bytes(Vec<u8>),
    Json(String),
    FileRef(DomainFileRef),
    Enum {
        type_id: u32,
        label:   String,
    },
    /// Ordinal-tagged composite fields `Vec<(slot, value)>`.
    Composite(Vec<(u16, DomainValue)>),
    /// Homogeneous or composite array elements.
    Array(Vec<DomainValue>),
    /// Tabular row set (`SETOF` / `RETURNS TABLE` / query rows).
    RowSet(Vec<Vec<(u16, DomainValue)>>),
}

/// Decoded current-epoch domain frame header + value payload.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainFrame {
    pub epoch:               u16,
    pub codec_version:       u16,
    pub kind:                DomainMessageKind,
    pub flags:               u8,
    pub descriptor_short_id: u16,
    pub payload:             DomainValue,
}

/// Resource budget enforced during domain frame decoding (`FR-049`).
#[derive(Debug, Clone, Copy)]
pub struct DomainDecodeBudget {
    pub max_wire_bytes:    usize,
    pub max_decoded_bytes: usize,
    pub max_elements:      usize,
    pub max_depth:         usize,
    pub max_work:          usize,
}

impl Default for DomainDecodeBudget {
    fn default() -> Self {
        Self {
            max_wire_bytes:    DEFAULT_MAX_FRAME_WIRE_BYTES,
            max_decoded_bytes: DEFAULT_MAX_DOMAIN_DECODED_BYTES,
            max_elements:      DEFAULT_MAX_DOMAIN_ELEMENTS,
            max_depth:         DEFAULT_MAX_DOMAIN_DEPTH,
            max_work:          DEFAULT_MAX_DOMAIN_WORK,
        }
    }
}

/// Encode a [`DomainValue`] into raw domain value bytes (without outer frame header).
pub fn encode_domain_value(value: &DomainValue) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(64);
    write_domain_value(&mut buf, value)?;
    Ok(buf)
}

/// Decode a [`DomainValue`] from raw domain value bytes under `budget`.
pub fn decode_domain_value(bytes: &[u8], budget: DomainDecodeBudget) -> Result<DomainValue> {
    if bytes.len() > budget.max_wire_bytes {
        return Err(SerializationError::Decode(format!(
            "domain payload length {} exceeds wire budget {}",
            bytes.len(),
            budget.max_wire_bytes
        )));
    }
    let mut reader = DomainReader::new(bytes, budget);
    let value = read_domain_value(&mut reader)?;
    if !reader.is_empty() {
        return Err(SerializationError::Decode(format!(
            "trailing {} byte(s) after domain value",
            reader.remaining()
        )));
    }
    Ok(value)
}

/// Encode a complete [`DomainFrame`] with `KDOM` header, epoch, kind, and payload.
pub fn encode_domain_frame(
    kind: DomainMessageKind,
    descriptor_short_id: u16,
    flags: u8,
    payload: &DomainValue,
) -> Result<Vec<u8>> {
    if flags != 0 {
        return Err(SerializationError::Encode(format!(
            "unsupported domain frame flags 0x{flags:02x}"
        )));
    }
    let mut buf = Vec::with_capacity(DOMAIN_FRAME_HEADER_LEN + 64);
    buf.extend_from_slice(&DOMAIN_MAGIC);
    buf.extend_from_slice(&DOMAIN_EPOCH.to_le_bytes());
    buf.extend_from_slice(&DOMAIN_CODEC_VERSION.to_le_bytes());
    buf.push(kind as u8);
    buf.push(flags);
    buf.extend_from_slice(&descriptor_short_id.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // placeholder for payload_len
    write_domain_value(&mut buf, payload)?;
    let payload_len = u32::try_from(buf.len() - DOMAIN_FRAME_HEADER_LEN)
        .map_err(|_| SerializationError::Encode("domain frame payload exceeds u32".to_string()))?;
    buf[12..16].copy_from_slice(&payload_len.to_le_bytes());
    Ok(buf)
}

/// Encode a frame whose payload bytes are already a complete [`DomainValue`].
///
/// Callers splice a cached row value into a per-subscriber frame without walking the row again.
pub fn encode_domain_frame_from_payload(
    kind: DomainMessageKind,
    descriptor_short_id: u16,
    flags: u8,
    payload: &[u8],
) -> Result<Vec<u8>> {
    if flags != 0 {
        return Err(SerializationError::Encode(format!(
            "unsupported domain frame flags 0x{flags:02x}"
        )));
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| {
        SerializationError::Encode("domain frame payload exceeds u32".to_string())
    })?;
    let mut buf = Vec::with_capacity(DOMAIN_FRAME_HEADER_LEN + payload.len());
    buf.extend_from_slice(&DOMAIN_MAGIC);
    buf.extend_from_slice(&DOMAIN_EPOCH.to_le_bytes());
    buf.extend_from_slice(&DOMAIN_CODEC_VERSION.to_le_bytes());
    buf.push(kind as u8);
    buf.push(flags);
    buf.extend_from_slice(&descriptor_short_id.to_le_bytes());
    buf.extend_from_slice(&payload_len.to_le_bytes());
    buf.extend_from_slice(payload);
    Ok(buf)
}

/// Decode a [`DomainFrame`] and enforce current-epoch, unsupported-format, and surface allowlist
/// rules.
pub fn decode_domain_frame_for_surface(
    bytes: &[u8],
    surface: EndpointSurface,
    budget: DomainDecodeBudget,
) -> Result<DomainFrame> {
    if bytes.len() > budget.max_wire_bytes {
        return Err(SerializationError::Decode(format!(
            "frame length {} exceeds wire budget {}",
            bytes.len(),
            budget.max_wire_bytes
        )));
    }
    // Explicitly detect legacy MessagePack map/array/fixstr headers (`0x80..=0x9f`, `0xdc..=0xdf`)
    // and legacy KOBJ envelopes on the new domain frame surface before allocation (`FR-052`,
    // `FR-061`).
    if let Some(&first) = bytes.first() {
        if matches!(first, 0x80..=0x9f | 0xdc..=0xdf) {
            return Err(SerializationError::UnsupportedFormat(
                "legacy MessagePack frames are not supported in the current epoch".to_string(),
            ));
        }
    }
    if bytes.starts_with(b"KOBJ") {
        return Err(SerializationError::EpochMismatch {
            expected: DOMAIN_EPOCH,
            found:    0,
        });
    }
    if bytes.len() < DOMAIN_FRAME_HEADER_LEN {
        return Err(SerializationError::Truncated);
    }
    if bytes[0..4] != DOMAIN_MAGIC {
        return Err(SerializationError::InvalidMagic);
    }
    let epoch = u16::from_le_bytes([bytes[4], bytes[5]]);
    if epoch != DOMAIN_EPOCH {
        return Err(SerializationError::EpochMismatch {
            expected: DOMAIN_EPOCH,
            found:    epoch,
        });
    }
    let codec_version = u16::from_le_bytes([bytes[6], bytes[7]]);
    if codec_version != DOMAIN_CODEC_VERSION {
        return Err(SerializationError::UnsupportedProtocolVersion {
            found:     codec_version,
            supported: DOMAIN_CODEC_VERSION,
        });
    }
    let kind = DomainMessageKind::from_u8(bytes[8])?;
    if !kind.is_allowed_on(surface) {
        return Err(SerializationError::ForbiddenMessageKind {
            kind:    bytes[8],
            surface: format!("{surface:?}"),
        });
    }
    let flags = bytes[9];
    if flags != 0 {
        return Err(SerializationError::Decode(format!(
            "unsupported domain frame flags 0x{flags:02x}"
        )));
    }
    let descriptor_short_id = u16::from_le_bytes([bytes[10], bytes[11]]);
    let payload_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let body = &bytes[DOMAIN_FRAME_HEADER_LEN..];
    if body.len() != payload_len {
        return Err(SerializationError::Decode(format!(
            "domain frame payload length mismatch: header declared {payload_len}, actual {}",
            body.len()
        )));
    }
    let payload = decode_domain_value(body, budget)?;
    Ok(DomainFrame {
        epoch,
        codec_version,
        kind,
        flags,
        descriptor_short_id,
        payload,
    })
}

/// Compute a SHA-256 32-byte digest (`T052`, `T070`).
pub fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Compute a lowercase 64-character SHA-256 hex digest (`T052`, `T070`).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha256_digest(bytes);
    digest_to_hex(&digest)
}

/// Format a 32-byte digest as a 64-character lowercase hex string.
pub fn digest_to_hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Compute a canonical SHA-256 digest of supplied invocation arguments and presence before
/// volatile defaults are evaluated (`FR-021`, `T034`, `T070`).
pub fn canonical_invocation_digest(
    epoch: u16,
    descriptor_revision: &[u8; 32],
    supplied_arguments: &[(u16, DomainValue)],
) -> Result<[u8; 32]> {
    let mut sorted: Vec<(u16, &DomainValue)> =
        supplied_arguments.iter().map(|(slot, val)| (*slot, val)).collect();
    sorted.sort_by_key(|(slot, _)| *slot);
    for window in sorted.windows(2) {
        if window[0].0 == window[1].0 {
            return Err(SerializationError::Encode(format!(
                "duplicate argument slot {} in canonical digest",
                window[0].0
            )));
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(b"KALAM_INVOCATION_DIGEST_V1");
    hasher.update(epoch.to_le_bytes());
    hasher.update(descriptor_revision);
    hasher.update((sorted.len() as u32).to_le_bytes());
    let mut val_buf = Vec::new();
    for (slot, val) in sorted {
        hasher.update(slot.to_le_bytes());
        val_buf.clear();
        write_domain_value(&mut val_buf, val)?;
        hasher.update((val_buf.len() as u32).to_le_bytes());
        hasher.update(&val_buf);
    }
    Ok(hasher.finalize().into())
}

pub(crate) fn write_domain_value(buf: &mut Vec<u8>, value: &DomainValue) -> Result<()> {
    match value {
        DomainValue::Omitted => buf.push(DTAG_OMITTED),
        DomainValue::SqlNull => buf.push(DTAG_SQL_NULL),
        DomainValue::JsonNull => buf.push(DTAG_JSON_NULL),
        DomainValue::Void => buf.push(DTAG_VOID),
        DomainValue::Bool(v) => {
            buf.push(DTAG_BOOL);
            buf.push(u8::from(*v));
        },
        DomainValue::Int8(v) => {
            buf.push(DTAG_I8);
            buf.push(*v as u8);
        },
        DomainValue::Int16(v) => {
            buf.push(DTAG_I16);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::Int32(v) => {
            buf.push(DTAG_I32);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::Int64(v) => {
            buf.push(DTAG_I64);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::UInt8(v) => {
            buf.push(DTAG_U8);
            buf.push(*v);
        },
        DomainValue::UInt16(v) => {
            buf.push(DTAG_U16);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::UInt32(v) => {
            buf.push(DTAG_U32);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::UInt64(v) => {
            buf.push(DTAG_U64);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::Float32(v) => {
            buf.push(DTAG_F32);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::Float64(v) => {
            buf.push(DTAG_F64);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::Decimal128 {
            unscaled,
            precision,
            scale,
        } => {
            buf.push(DTAG_DECIMAL128);
            buf.push(*precision);
            buf.push(*scale as u8);
            buf.extend_from_slice(&unscaled.to_le_bytes());
        },
        DomainValue::Date32(v) => {
            buf.push(DTAG_DATE32);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::TimeMicros(v) => {
            buf.push(DTAG_TIME_MICROS);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::TimestampMicros(v) => {
            buf.push(DTAG_TIMESTAMP_MICROS);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::TimestampTzMicros(v) => {
            buf.push(DTAG_TIMESTAMPTZ_MICROS);
            buf.extend_from_slice(&v.to_le_bytes());
        },
        DomainValue::Uuid(bytes) => {
            buf.push(DTAG_UUID);
            buf.extend_from_slice(bytes);
        },
        DomainValue::Utf8(s) => {
            buf.push(DTAG_UTF8);
            write_len_bytes(buf, s.as_bytes())?;
        },
        DomainValue::Bytes(bytes) => {
            buf.push(DTAG_BYTES);
            write_len_bytes(buf, bytes)?;
        },
        DomainValue::Json(json_text) => {
            buf.push(DTAG_JSON);
            write_len_bytes(buf, json_text.as_bytes())?;
        },
        DomainValue::FileRef(file) => {
            buf.push(DTAG_FILE_REF);
            write_len_bytes(buf, file.id.as_bytes())?;
            write_len_bytes(buf, file.sub.as_bytes())?;
            write_len_bytes(buf, file.name.as_bytes())?;
            buf.extend_from_slice(&file.size.to_le_bytes());
            write_len_bytes(buf, file.mime.as_bytes())?;
            write_len_bytes(buf, file.sha256.as_bytes())?;
            match file.shard {
                Some(shard) => {
                    buf.push(1);
                    buf.extend_from_slice(&shard.to_le_bytes());
                },
                None => buf.push(0),
            }
        },
        DomainValue::Enum { type_id, label } => {
            buf.push(DTAG_ENUM);
            buf.extend_from_slice(&type_id.to_le_bytes());
            write_len_bytes(buf, label.as_bytes())?;
        },
        DomainValue::Composite(fields) => {
            buf.push(DTAG_COMPOSITE);
            let count = u16::try_from(fields.len()).map_err(|_| {
                SerializationError::Encode("composite field count exceeds u16".to_string())
            })?;
            buf.extend_from_slice(&count.to_le_bytes());
            let mut prev_slot = None;
            for (slot, field_val) in fields {
                if let Some(prev) = prev_slot {
                    if *slot <= prev {
                        return Err(SerializationError::Encode(format!(
                            "composite slots must be strictly increasing ({prev} then {slot})"
                        )));
                    }
                }
                prev_slot = Some(*slot);
                buf.extend_from_slice(&slot.to_le_bytes());
                write_domain_value(buf, field_val)?;
            }
        },
        DomainValue::Array(items) => {
            buf.push(DTAG_ARRAY);
            let count = u32::try_from(items.len())
                .map_err(|_| SerializationError::Encode("array length exceeds u32".to_string()))?;
            buf.extend_from_slice(&count.to_le_bytes());
            for item in items {
                write_domain_value(buf, item)?;
            }
        },
        DomainValue::RowSet(rows) => {
            buf.push(DTAG_ROW_SET);
            let row_count = u32::try_from(rows.len())
                .map_err(|_| SerializationError::Encode("rowset length exceeds u32".to_string()))?;
            buf.extend_from_slice(&row_count.to_le_bytes());
            for row in rows {
                let col_count = u16::try_from(row.len()).map_err(|_| {
                    SerializationError::Encode("row column count exceeds u16".to_string())
                })?;
                buf.extend_from_slice(&col_count.to_le_bytes());
                for (slot, cell) in row {
                    buf.extend_from_slice(&slot.to_le_bytes());
                    write_domain_value(buf, cell)?;
                }
            }
        },
    }
    Ok(())
}

#[inline]
fn write_len_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| SerializationError::Encode("byte slice exceeds u32 length".to_string()))?;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(bytes);
    Ok(())
}

struct DomainReader<'a> {
    rest:          &'a [u8],
    budget:        DomainDecodeBudget,
    depth:         usize,
    work:          usize,
    decoded_bytes: usize,
    elements:      usize,
}

impl<'a> DomainReader<'a> {
    fn new(bytes: &'a [u8], budget: DomainDecodeBudget) -> Self {
        Self {
            rest: bytes,
            budget,
            depth: 0,
            work: 0,
            decoded_bytes: 0,
            elements: 0,
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.rest.len()
    }

    #[inline]
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.rest.len() < n {
            return Err(SerializationError::Truncated);
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    #[inline]
    fn charge_work(&mut self, units: usize) -> Result<()> {
        let next = self.work.checked_add(units).ok_or_else(|| {
            SerializationError::Decode("domain decode work budget overflow".to_string())
        })?;
        if next > self.budget.max_work {
            return Err(SerializationError::Decode(format!(
                "domain decode work budget exceeded ({next} > {})",
                self.budget.max_work
            )));
        }
        self.work = next;
        Ok(())
    }

    #[inline]
    fn charge_elements(&mut self, count: usize) -> Result<()> {
        let next = self.elements.checked_add(count).ok_or_else(|| {
            SerializationError::Decode("domain element count overflow".to_string())
        })?;
        if next > self.budget.max_elements {
            return Err(SerializationError::Decode(format!(
                "domain element count exceeded ({next} > {})",
                self.budget.max_elements
            )));
        }
        self.elements = next;
        Ok(())
    }

    #[inline]
    fn charge_bytes(&mut self, bytes: usize) -> Result<()> {
        let next = self.decoded_bytes.checked_add(bytes).ok_or_else(|| {
            SerializationError::Decode("domain decoded byte budget overflow".to_string())
        })?;
        if next > self.budget.max_decoded_bytes {
            return Err(SerializationError::Decode(format!(
                "domain decoded byte budget exceeded ({next} > {})",
                self.budget.max_decoded_bytes
            )));
        }
        self.decoded_bytes = next;
        Ok(())
    }

    #[inline]
    fn enter_nested(&mut self) -> Result<()> {
        let next = self.depth.checked_add(1).ok_or_else(|| {
            SerializationError::Decode("domain nesting depth overflow".to_string())
        })?;
        if next > self.budget.max_depth {
            return Err(SerializationError::Decode(format!(
                "domain value exceeds max nesting depth {}",
                self.budget.max_depth
            )));
        }
        self.depth = next;
        Ok(())
    }

    #[inline]
    fn exit_nested(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    #[inline]
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    #[inline]
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    #[inline]
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline]
    fn i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline]
    fn i64(&mut self) -> Result<i64> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    #[inline]
    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    #[inline]
    fn len_bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        if len > self.remaining() {
            return Err(SerializationError::Truncated);
        }
        self.charge_bytes(len)?;
        self.take(len)
    }

    #[inline]
    fn string(&mut self) -> Result<String> {
        let bytes = self.len_bytes()?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|err| SerializationError::Decode(format!("invalid utf8: {err}")))
    }
}

fn read_domain_value(reader: &mut DomainReader<'_>) -> Result<DomainValue> {
    reader.charge_work(1)?;
    let tag = reader.u8()?;
    match tag {
        DTAG_OMITTED => Ok(DomainValue::Omitted),
        DTAG_SQL_NULL => Ok(DomainValue::SqlNull),
        DTAG_JSON_NULL => Ok(DomainValue::JsonNull),
        DTAG_VOID => Ok(DomainValue::Void),
        DTAG_BOOL => match reader.u8()? {
            0 => Ok(DomainValue::Bool(false)),
            1 => Ok(DomainValue::Bool(true)),
            other => Err(SerializationError::Decode(format!(
                "invalid boolean byte {other}; expected 0 or 1"
            ))),
        },
        DTAG_I8 => Ok(DomainValue::Int8(reader.u8()? as i8)),
        DTAG_I16 => Ok(DomainValue::Int16(reader.u16()? as i16)),
        DTAG_I32 => Ok(DomainValue::Int32(reader.i32()?)),
        DTAG_I64 => Ok(DomainValue::Int64(reader.i64()?)),
        DTAG_U8 => Ok(DomainValue::UInt8(reader.u8()?)),
        DTAG_U16 => Ok(DomainValue::UInt16(reader.u16()?)),
        DTAG_U32 => Ok(DomainValue::UInt32(reader.u32()?)),
        DTAG_U64 => Ok(DomainValue::UInt64(reader.u64()?)),
        DTAG_F32 => {
            let b = reader.take(4)?;
            Ok(DomainValue::Float32(f32::from_le_bytes([b[0], b[1], b[2], b[3]])))
        },
        DTAG_F64 => {
            let b = reader.take(8)?;
            Ok(DomainValue::Float64(f64::from_le_bytes([
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
            ])))
        },
        DTAG_DECIMAL128 => {
            let precision = reader.u8()?;
            let scale = reader.u8()? as i8;
            if precision == 0 || precision > 38 || scale < 0 || (scale as u8) > precision {
                return Err(SerializationError::Decode(format!(
                    "invalid decimal128 metadata ({precision}, {scale})"
                )));
            }
            let b = reader.take(16)?;
            let mut arr = [0u8; 16];
            arr.copy_from_slice(b);
            Ok(DomainValue::Decimal128 {
                unscaled: i128::from_le_bytes(arr),
                precision,
                scale,
            })
        },
        DTAG_DATE32 => Ok(DomainValue::Date32(reader.i32()?)),
        DTAG_TIME_MICROS => Ok(DomainValue::TimeMicros(reader.i64()?)),
        DTAG_TIMESTAMP_MICROS => Ok(DomainValue::TimestampMicros(reader.i64()?)),
        DTAG_TIMESTAMPTZ_MICROS => Ok(DomainValue::TimestampTzMicros(reader.i64()?)),
        DTAG_UUID => {
            let b = reader.take(16)?;
            let mut out = [0u8; 16];
            out.copy_from_slice(b);
            Ok(DomainValue::Uuid(out))
        },
        DTAG_UTF8 => Ok(DomainValue::Utf8(reader.string()?)),
        DTAG_BYTES => Ok(DomainValue::Bytes(reader.len_bytes()?.to_vec())),
        DTAG_JSON => Ok(DomainValue::Json(reader.string()?)),
        DTAG_FILE_REF => {
            let id = reader.string()?;
            let sub = reader.string()?;
            let name = reader.string()?;
            let size = reader.i64()?;
            let mime = reader.string()?;
            let sha256 = reader.string()?;
            let shard = match reader.u8()? {
                0 => None,
                1 => Some(reader.i32()?),
                other => {
                    return Err(SerializationError::Decode(format!(
                        "invalid file_ref shard presence tag {other}"
                    )))
                },
            };
            Ok(DomainValue::FileRef(DomainFileRef {
                id,
                sub,
                name,
                size,
                mime,
                sha256,
                shard,
            }))
        },
        DTAG_ENUM => {
            let type_id = reader.u32()?;
            let label = reader.string()?;
            Ok(DomainValue::Enum { type_id, label })
        },
        DTAG_COMPOSITE => {
            reader.enter_nested()?;
            let res = (|| {
                let count = reader.u16()? as usize;
                // Each composite entry is at least 2 bytes (slot) + 1 byte (value tag) = 3 bytes.
                if count.saturating_mul(3) > reader.remaining() {
                    return Err(SerializationError::Truncated);
                }
                reader.charge_elements(count)?;
                reader.charge_bytes(count.saturating_mul(24))?;
                let mut fields = Vec::with_capacity(count.min(MAX_INITIAL_CAPACITY));
                let mut prev_slot = None;
                for _ in 0..count {
                    let slot = reader.u16()?;
                    if let Some(prev) = prev_slot {
                        if slot <= prev {
                            return Err(SerializationError::Decode(format!(
                                "duplicate or unsorted composite slot {slot} after {prev}"
                            )));
                        }
                    }
                    prev_slot = Some(slot);
                    let val = read_domain_value(reader)?;
                    fields.push((slot, val));
                }
                Ok(DomainValue::Composite(fields))
            })();
            reader.exit_nested();
            res
        },
        DTAG_ARRAY => {
            reader.enter_nested()?;
            let res = (|| {
                let len = reader.u32()? as usize;
                if len > reader.remaining() {
                    return Err(SerializationError::Truncated);
                }
                reader.charge_elements(len)?;
                reader.charge_bytes(len.saturating_mul(16))?;
                let mut items = Vec::with_capacity(len.min(MAX_INITIAL_CAPACITY));
                for _ in 0..len {
                    items.push(read_domain_value(reader)?);
                }
                Ok(DomainValue::Array(items))
            })();
            reader.exit_nested();
            res
        },
        DTAG_ROW_SET => {
            reader.enter_nested()?;
            let res = (|| {
                let row_count = reader.u32()? as usize;
                if row_count.saturating_mul(2) > reader.remaining() {
                    return Err(SerializationError::Truncated);
                }
                reader.charge_elements(row_count)?;
                let mut rows = Vec::with_capacity(row_count.min(MAX_INITIAL_CAPACITY));
                for _ in 0..row_count {
                    let col_count = reader.u16()? as usize;
                    if col_count.saturating_mul(3) > reader.remaining() {
                        return Err(SerializationError::Truncated);
                    }
                    reader.charge_elements(col_count)?;
                    let mut row = Vec::with_capacity(col_count.min(MAX_INITIAL_CAPACITY));
                    for _ in 0..col_count {
                        let slot = reader.u16()?;
                        let cell = read_domain_value(reader)?;
                        row.push((slot, cell));
                    }
                    rows.push(row);
                }
                Ok(DomainValue::RowSet(rows))
            })();
            reader.exit_nested();
            res
        },
        other => Err(SerializationError::Decode(format!("unknown domain value tag 0x{other:02x}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t070_golden_vectors_and_presence_distinction() {
        // Golden vector 1: 4 distinct presence states (Omitted, SqlNull, JsonNull, Void)
        assert_eq!(encode_domain_value(&DomainValue::Omitted).unwrap(), vec![0x00]);
        assert_eq!(encode_domain_value(&DomainValue::SqlNull).unwrap(), vec![0x01]);
        assert_eq!(encode_domain_value(&DomainValue::JsonNull).unwrap(), vec![0x02]);
        assert_eq!(encode_domain_value(&DomainValue::Void).unwrap(), vec![0x03]);

        // Golden vector 2: RPC request frame with composite input
        let input = DomainValue::Composite(vec![
            (1, DomainValue::Int64(9_007_199_254_740_993)),
            (
                2,
                DomainValue::Decimal128 {
                    unscaled:  -123_456_789,
                    precision: 18,
                    scale:     4,
                },
            ),
            (3, DomainValue::TimestampTzMicros(1_792_793_625_123_456)),
            (4, DomainValue::Omitted),
            (5, DomainValue::SqlNull),
            (6, DomainValue::JsonNull),
        ]);
        let frame_bytes =
            encode_domain_frame(DomainMessageKind::RpcRequest, 42, 0, &input).unwrap();
        assert_eq!(&frame_bytes[0..4], b"KDOM");
        let decoded = decode_domain_frame_for_surface(
            &frame_bytes,
            EndpointSurface::PublicRpc,
            DomainDecodeBudget::default(),
        )
        .unwrap();
        assert_eq!(decoded.epoch, DOMAIN_EPOCH);
        assert_eq!(decoded.kind, DomainMessageKind::RpcRequest);
        assert_eq!(decoded.descriptor_short_id, 42);
        assert_eq!(decoded.payload, input);
    }

    #[test]
    fn t070_rejects_wrong_surface_legacy_msgpack_and_old_epoch() {
        // Internal Raft frame rejected on PublicRpc and WebSocket surfaces
        let raft_frame =
            encode_domain_frame(DomainMessageKind::RaftCommand, 1, 0, &DomainValue::Int32(7))
                .unwrap();
        let err = decode_domain_frame_for_surface(
            &raft_frame,
            EndpointSurface::PublicRpc,
            DomainDecodeBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(err, SerializationError::ForbiddenMessageKind { .. }), "{err}");

        let err_ws = decode_domain_frame_for_surface(
            &raft_frame,
            EndpointSurface::WebSocket,
            DomainDecodeBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(err_ws, SerializationError::ForbiddenMessageKind { .. }), "{err_ws}");

        // Allowed on InternalRaft
        assert!(decode_domain_frame_for_surface(
            &raft_frame,
            EndpointSurface::InternalRaft,
            DomainDecodeBudget::default(),
        )
        .is_ok());

        // Legacy MessagePack fixmap (0x82) rejected with UnsupportedFormat
        let msgpack_bytes = [
            0x82, 0xa4, b't', b'y', b'p', b'e', 0xa4, b'p', b'i', b'n', b'g',
        ];
        let err_mp = decode_domain_frame_for_surface(
            &msgpack_bytes,
            EndpointSurface::WebSocket,
            DomainDecodeBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(err_mp, SerializationError::UnsupportedFormat(_)), "{err_mp}");

        // Mismatched epoch (e.g. epoch = 99) rejected with EpochMismatch
        let mut future_epoch =
            encode_domain_frame(DomainMessageKind::WsPing, 0, 0, &DomainValue::Void).unwrap();
        future_epoch[4..6].copy_from_slice(&99u16.to_le_bytes());
        let err_epoch = decode_domain_frame_for_surface(
            &future_epoch,
            EndpointSurface::WebSocket,
            DomainDecodeBudget::default(),
        )
        .unwrap_err();
        assert_eq!(
            err_epoch,
            SerializationError::EpochMismatch {
                expected: DOMAIN_EPOCH,
                found:    99,
            }
        );
    }

    #[test]
    fn t070_canonical_digest_is_order_independent_and_distinguishes_omitted_vs_null() {
        let rev = [0x11u8; 32];
        let d1 = canonical_invocation_digest(
            DOMAIN_EPOCH,
            &rev,
            &[
                (2, DomainValue::Utf8("hello".into())),
                (1, DomainValue::Omitted),
            ],
        )
        .unwrap();
        let d2 = canonical_invocation_digest(
            DOMAIN_EPOCH,
            &rev,
            &[
                (1, DomainValue::Omitted),
                (2, DomainValue::Utf8("hello".into())),
            ],
        )
        .unwrap();
        assert_eq!(d1, d2, "slot ordering must canonicalize");

        let d_null = canonical_invocation_digest(
            DOMAIN_EPOCH,
            &rev,
            &[
                (1, DomainValue::SqlNull),
                (2, DomainValue::Utf8("hello".into())),
            ],
        )
        .unwrap();
        assert_ne!(d1, d_null, "Omitted and SqlNull must produce distinct digests");
    }

    #[test]
    fn t082_gate_d_codec_candidate_comparison_workload_suite() {
        use std::time::Instant;

        // Workload W1+W2: dynamic SQL contract payload with wide int64, exact decimal128,
        // microsecond timestamptz, 4-state presence, and nested composite/array fields.
        let sample = DomainValue::Composite(vec![
            (1, DomainValue::Int64(9_007_199_254_740_993)),
            (
                2,
                DomainValue::Decimal128 {
                    unscaled:  123_456_789_012_345,
                    precision: 18,
                    scale:     4,
                },
            ),
            (3, DomainValue::TimestampTzMicros(1_792_793_625_123_456)),
            (4, DomainValue::Utf8("school.enrollments".to_string())),
            (
                5,
                DomainValue::Array(vec![
                    DomainValue::Utf8("owner".to_string()),
                    DomainValue::Utf8("teacher".to_string()),
                    DomainValue::Utf8("parent".to_string()),
                ]),
            ),
            (6, DomainValue::Omitted),
            (7, DomainValue::SqlNull),
            (8, DomainValue::JsonNull),
        ]);

        let iterations = 5_000usize;

        // Candidate 1: Hardened Kalam schema-directed packed domain codec (`kalam-packed`)
        let t0 = Instant::now();
        let mut kalam_bytes_len = 0usize;
        for _ in 0..iterations {
            let encoded =
                encode_domain_frame(DomainMessageKind::RpcRequest, 7, 0, &sample).unwrap();
            kalam_bytes_len = encoded.len();
            let decoded = decode_domain_frame_for_surface(
                &encoded,
                EndpointSurface::PublicRpc,
                DomainDecodeBudget::default(),
            )
            .unwrap();
            std::hint::black_box(decoded);
        }
        let kalam_secs = t0.elapsed().as_secs_f64();

        // Baseline 1: JSON (`serde_json`) over the same semantic model
        let t1 = Instant::now();
        let mut json_bytes_len = 0usize;
        for _ in 0..iterations {
            let encoded = serde_json::to_vec(&sample).unwrap();
            json_bytes_len = encoded.len();
            let decoded: DomainValue = serde_json::from_slice(&encoded).unwrap();
            std::hint::black_box(decoded);
        }
        let json_secs = t1.elapsed().as_secs_f64();

        // Candidate 2: Postcard-style varint/packed encoding over the same dynamic SQL contract
        let t2 = Instant::now();
        let mut postcard_bytes_len = 0usize;
        for _ in 0..iterations {
            let encoded = encode_postcard_candidate(&sample);
            postcard_bytes_len = encoded.len();
            let decoded = decode_postcard_candidate(&encoded).unwrap();
            std::hint::black_box(decoded);
        }
        let postcard_secs = t2.elapsed().as_secs_f64();

        // Candidate 3: Prost-style dynamic protobuf tag/wire encoding over the same dynamic SQL
        // contract
        let t3 = Instant::now();
        let mut prost_bytes_len = 0usize;
        for _ in 0..iterations {
            let encoded = encode_prost_dynamic_candidate(&sample);
            prost_bytes_len = encoded.len();
            let decoded = decode_prost_dynamic_candidate(&encoded).unwrap();
            std::hint::black_box(decoded);
        }
        let prost_secs = t3.elapsed().as_secs_f64();

        // Failed case record: FlexBuffers Serde fails on native i128 (`Decimal128`) without a
        // custom wrapper
        let flex_err = crate::object::encode_flexbuffers(&sample).unwrap_err();
        assert!(flex_err.to_string().contains("i128"), "{flex_err}");

        eprintln!(
            "Gate D (5000 iters): kalam-packed={kalam_secs:.4}s ({kalam_bytes_len}B), \
             postcard={postcard_secs:.4}s ({postcard_bytes_len}B), prost-dynamic={prost_secs:.4}s \
             ({prost_bytes_len}B), json={json_secs:.4}s ({json_bytes_len}B)"
        );
        assert!(kalam_bytes_len < json_bytes_len);
        assert!(kalam_bytes_len <= prost_bytes_len);
    }

    fn encode_varint_u64(buf: &mut Vec<u8>, mut val: u64) {
        while val >= 0x80 {
            buf.push(((val as u8) & 0x7f) | 0x80);
            val >>= 7;
        }
        buf.push(val as u8);
    }

    fn decode_varint_u64(bytes: &[u8], pos: &mut usize) -> Result<u64> {
        let mut result = 0u64;
        let mut shift = 0u32;
        while *pos < bytes.len() && shift < 64 {
            let b = bytes[*pos];
            *pos += 1;
            result |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
        Err(SerializationError::Truncated)
    }

    fn encode_postcard_candidate(val: &DomainValue) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        write_postcard_val(&mut buf, val);
        buf
    }

    fn write_postcard_val(buf: &mut Vec<u8>, val: &DomainValue) {
        match val {
            DomainValue::Omitted => encode_varint_u64(buf, 0),
            DomainValue::SqlNull => encode_varint_u64(buf, 1),
            DomainValue::JsonNull => encode_varint_u64(buf, 2),
            DomainValue::Int64(v) => {
                encode_varint_u64(buf, 8);
                let zigzag = ((*v << 1) ^ (*v >> 63)) as u64;
                encode_varint_u64(buf, zigzag);
            },
            DomainValue::Decimal128 {
                unscaled,
                precision,
                scale,
            } => {
                encode_varint_u64(buf, 15);
                buf.push(*precision);
                buf.push(*scale as u8);
                buf.extend_from_slice(&unscaled.to_le_bytes());
            },
            DomainValue::TimestampTzMicros(v) => {
                encode_varint_u64(buf, 19);
                let zigzag = ((*v << 1) ^ (*v >> 63)) as u64;
                encode_varint_u64(buf, zigzag);
            },
            DomainValue::Utf8(s) => {
                encode_varint_u64(buf, 21);
                encode_varint_u64(buf, s.len() as u64);
                buf.extend_from_slice(s.as_bytes());
            },
            DomainValue::Array(items) => {
                encode_varint_u64(buf, 27);
                encode_varint_u64(buf, items.len() as u64);
                for item in items {
                    write_postcard_val(buf, item);
                }
            },
            DomainValue::Composite(fields) => {
                encode_varint_u64(buf, 26);
                encode_varint_u64(buf, fields.len() as u64);
                for (slot, fval) in fields {
                    encode_varint_u64(buf, u64::from(*slot));
                    write_postcard_val(buf, fval);
                }
            },
            _ => encode_varint_u64(buf, 3),
        }
    }

    fn decode_postcard_candidate(bytes: &[u8]) -> Result<DomainValue> {
        let mut pos = 0usize;
        read_postcard_val(bytes, &mut pos)
    }

    fn read_postcard_val(bytes: &[u8], pos: &mut usize) -> Result<DomainValue> {
        let disc = decode_varint_u64(bytes, pos)?;
        match disc {
            0 => Ok(DomainValue::Omitted),
            1 => Ok(DomainValue::SqlNull),
            2 => Ok(DomainValue::JsonNull),
            8 => {
                let zz = decode_varint_u64(bytes, pos)?;
                let v = ((zz >> 1) as i64) ^ (-((zz & 1) as i64));
                Ok(DomainValue::Int64(v))
            },
            15 => {
                let p = bytes[*pos];
                let s = bytes[*pos + 1] as i8;
                *pos += 2;
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&bytes[*pos..*pos + 16]);
                *pos += 16;
                Ok(DomainValue::Decimal128 {
                    unscaled:  i128::from_le_bytes(arr),
                    precision: p,
                    scale:     s,
                })
            },
            19 => {
                let zz = decode_varint_u64(bytes, pos)?;
                let v = ((zz >> 1) as i64) ^ (-((zz & 1) as i64));
                Ok(DomainValue::TimestampTzMicros(v))
            },
            21 => {
                let len = decode_varint_u64(bytes, pos)? as usize;
                let s = std::str::from_utf8(&bytes[*pos..*pos + len]).unwrap().to_owned();
                *pos += len;
                Ok(DomainValue::Utf8(s))
            },
            26 => {
                let count = decode_varint_u64(bytes, pos)? as usize;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    let slot = decode_varint_u64(bytes, pos)? as u16;
                    fields.push((slot, read_postcard_val(bytes, pos)?));
                }
                Ok(DomainValue::Composite(fields))
            },
            27 => {
                let count = decode_varint_u64(bytes, pos)? as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(read_postcard_val(bytes, pos)?);
                }
                Ok(DomainValue::Array(items))
            },
            _ => Ok(DomainValue::Void),
        }
    }

    fn encode_prost_dynamic_candidate(val: &DomainValue) -> Vec<u8> {
        let mut buf = Vec::with_capacity(96);
        write_prost_val(&mut buf, val);
        buf
    }

    fn write_prost_val(buf: &mut Vec<u8>, val: &DomainValue) {
        // Protobuf wire: tag = (field_number << 3) | wire_type, plus submessage length prefix
        match val {
            DomainValue::Composite(fields) => {
                encode_varint_u64(buf, (26 << 3) | 2);
                let mut inner = Vec::new();
                for (slot, fval) in fields {
                    encode_varint_u64(&mut inner, (1 << 3) | 2);
                    let mut entry = Vec::new();
                    encode_varint_u64(&mut entry, (1 << 3) | 0);
                    encode_varint_u64(&mut entry, u64::from(*slot));
                    encode_varint_u64(&mut entry, (2 << 3) | 2);
                    let mut vbytes = Vec::new();
                    write_prost_val(&mut vbytes, fval);
                    encode_varint_u64(&mut entry, vbytes.len() as u64);
                    entry.extend_from_slice(&vbytes);
                    encode_varint_u64(&mut inner, entry.len() as u64);
                    inner.extend_from_slice(&entry);
                }
                encode_varint_u64(buf, inner.len() as u64);
                buf.extend_from_slice(&inner);
            },
            _ => {
                // Delegate leaf encoding + protobuf length-delimited wrapper
                encode_varint_u64(buf, (1 << 3) | 2);
                let mut leaf = Vec::new();
                write_postcard_val(&mut leaf, val);
                encode_varint_u64(buf, leaf.len() as u64);
                buf.extend_from_slice(&leaf);
            },
        }
    }

    fn decode_prost_dynamic_candidate(bytes: &[u8]) -> Result<DomainValue> {
        let mut pos = 0usize;
        read_prost_val(bytes, &mut pos)
    }

    fn read_prost_val(bytes: &[u8], pos: &mut usize) -> Result<DomainValue> {
        let tag = decode_varint_u64(bytes, pos)?;
        let len = decode_varint_u64(bytes, pos)? as usize;
        let end = *pos + len;
        if (tag >> 3) == 26 {
            let mut fields = Vec::new();
            while *pos < end {
                let _entry_tag = decode_varint_u64(bytes, pos)?;
                let entry_len = decode_varint_u64(bytes, pos)? as usize;
                let _entry_end = *pos + entry_len;
                let _slot_tag = decode_varint_u64(bytes, pos)?;
                let slot = decode_varint_u64(bytes, pos)? as u16;
                let _val_tag = decode_varint_u64(bytes, pos)?;
                let _val_len = decode_varint_u64(bytes, pos)?;
                let val = read_prost_val(bytes, pos)?;
                fields.push((slot, val));
            }
            Ok(DomainValue::Composite(fields))
        } else {
            let val = read_postcard_val(&bytes[..end], pos)?;
            *pos = end;
            Ok(val)
        }
    }
}
