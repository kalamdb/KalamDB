//! Kalam Object Protocol envelope.
//!
//! Callers choose a semantic [`ObjectKind`]. Codec choice stays inside this crate.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    error::{Result, SerializationError},
    version::{MAGIC, PROTOCOL_VERSION},
};

/// Semantic persisted-object class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectKind {
    /// Generic structured catalog/system objects.
    Generic = 1,
    /// Hot table rows.
    Row = 2,
    /// Typed protocol objects such as Raft commands.
    Protocol = 3,
    /// Stream log records.
    Stream = 4,
    /// In-memory function JS-boundary values (not a durable catalog object).
    Function = 5,
}

impl ObjectKind {
    pub(crate) fn from_u8(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::Generic),
            2 => Ok(Self::Row),
            3 => Ok(Self::Protocol),
            4 => Ok(Self::Stream),
            5 => Ok(Self::Function),
            other => Err(SerializationError::Decode(format!("unknown object kind {other}"))),
        }
    }
}

/// Owned encoded object bytes. Does not re-serialize on access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedObject {
    bytes: Vec<u8>,
}

impl EncodedObject {
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Borrow the encoded bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Encoded length in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the object is empty (should not happen for a valid envelope).
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Consume and return the encoded bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Header parsed from a valid envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvelopeHeader {
    pub protocol_version: u16,
    pub object_kind: ObjectKind,
    pub schema_version: u16,
    pub flags: u16,
}

pub(crate) const HEADER_LEN: usize = 4 + 2 + 1 + 2 + 2 + 4;

/// True when `bytes` starts with the Kalam object magic (`KOBJ`).
pub fn has_object_magic(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC.len() && bytes[..MAGIC.len()] == MAGIC
}

/// Row payloads include a per-column offset table after `field_count`.
pub(crate) const FLAG_COLUMN_OFFSETS: u16 = 1 << 0;
/// Row payload stores the tombstone flag only. `_version` lives on the storage key.
pub(crate) const FLAG_VERSION_IN_KEY: u16 = 1 << 1;
/// Row payload uses the packed v2 fixed/variable body layout (`RowView`).
pub const FLAG_ROW_BODY_V2: u16 = 1 << 2;
/// Row payload is a tombstone-only body with no column values.
pub const FLAG_TOMBSTONE_ONLY: u16 = 1 << 3;

/// Supported flag mask for `ObjectKind::Row` envelopes.
pub const SUPPORTED_ROW_FLAGS: u16 =
    FLAG_COLUMN_OFFSETS | FLAG_VERSION_IN_KEY | FLAG_ROW_BODY_V2 | FLAG_TOMBSTONE_ONLY;

/// Row payload written by the original KOBJ row body.
pub const ROW_FORMAT_V1: u8 = 1;
/// Packed row body (`FLAG_ROW_BODY_V2`).
pub const ROW_FORMAT_V2: u8 = 2;

static ROW_V2_WRITERS_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// One replica's durable reader capability for row payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaFormatCapability {
    pub replica_id: String,
    pub eligible: bool,
    pub max_readable_row_format: u8,
}

/// Durable cluster barrier for row-format writers.
///
/// `secondary_index_format_version` stays independent of the row writer flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatCompatibilityFloor {
    pub min_supported_row_format: u8,
    pub row_v2_writer_enabled: bool,
    pub secondary_index_format_version: u8,
    pub replicas: Vec<ReplicaFormatCapability>,
}

/// Why a row v2 writer activation was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RowFormatActivationError {
    #[error("format compatibility floor is not installed")]
    FloorNotInstalled,
    #[error("no eligible replica has acknowledged row format {0}")]
    NoEligibleReplica(u8),
    #[error(
        "eligible replica {replica_id} reads row format {max_readable_row_format}, below required {required}"
    )]
    IncompatibleReplica {
        replica_id: String,
        max_readable_row_format: u8,
        required: u8,
    },
    #[error("failed to persist format compatibility floor: {0}")]
    Persist(String),
}

struct InstalledFormatFloor {
    floor: FormatCompatibilityFloor,
    path: PathBuf,
}

static FORMAT_FLOOR: Mutex<Option<InstalledFormatFloor>> = Mutex::new(None);

#[cfg(test)]
static ROW_FORMAT_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
std::thread_local! {
    static ROW_FORMAT_TEST_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Query whether row v2 writers are enabled past the durable format capability barrier.
/// Read-old/write-old deployment stage defaults to `false`.
#[inline]
pub fn is_row_v2_writer_enabled() -> bool {
    ROW_V2_WRITERS_ENABLED.load(std::sync::atomic::Ordering::Acquire)
}

/// The installed durable floor, if startup or an operator has loaded one.
pub fn format_compatibility_floor(
) -> std::result::Result<Option<FormatCompatibilityFloor>, RowFormatActivationError> {
    let guard = lock_floor()?;
    Ok(guard.as_ref().map(|installed| installed.floor.clone()))
}

/// Persist `floor` and install it as the process barrier.
///
/// A floor that claims the v2 writer is on while an eligible replica cannot read
/// format 2 is rejected and does not enable writers.
pub fn install_format_compatibility_floor(
    floor: FormatCompatibilityFloor,
    path: &Path,
) -> std::result::Result<(), RowFormatActivationError> {
    validate_floor(&floor, floor.row_v2_writer_enabled)?;
    persist_floor(&floor, path)?;
    store_installed_floor(floor, path.to_path_buf())
}

/// Load a previously persisted floor and apply its writer bit when it still validates.
pub fn load_format_compatibility_floor(
    path: &Path,
) -> std::result::Result<FormatCompatibilityFloor, RowFormatActivationError> {
    let bytes = fs::read(path).map_err(|err| RowFormatActivationError::Persist(err.to_string()))?;
    let floor: FormatCompatibilityFloor = serde_json::from_slice(&bytes)
        .map_err(|err| RowFormatActivationError::Persist(err.to_string()))?;
    validate_floor(&floor, floor.row_v2_writer_enabled)?;
    let loaded = floor.clone();
    store_installed_floor(floor, path.to_path_buf())?;
    Ok(loaded)
}

/// Enable or disable row v2 writers.
///
/// Enabling requires a durable floor where every eligible replica can read row
/// format 2. Disabling persists `row_v2_writer_enabled = false` and leaves
/// existing v1 and v2 rows readable. The secondary-index format version is not
/// changed.
pub fn set_row_v2_writer_enabled(
    enabled: bool,
) -> std::result::Result<(), RowFormatActivationError> {
    let mut guard = lock_floor()?;
    let Some(installed) = guard.as_mut() else {
        if enabled {
            return Err(RowFormatActivationError::FloorNotInstalled);
        }
        ROW_V2_WRITERS_ENABLED.store(false, std::sync::atomic::Ordering::Release);
        return Ok(());
    };
    let mut updated = installed.floor.clone();
    if enabled {
        validate_floor(&updated, true)?;
        updated.min_supported_row_format = updated.min_supported_row_format.max(ROW_FORMAT_V2);
    }
    updated.row_v2_writer_enabled = enabled;
    persist_floor(&updated, &installed.path)?;
    installed.floor = updated;
    ROW_V2_WRITERS_ENABLED.store(enabled, std::sync::atomic::Ordering::Release);
    Ok(())
}

fn lock_floor(
) -> std::result::Result<MutexGuard<'static, Option<InstalledFormatFloor>>, RowFormatActivationError>
{
    FORMAT_FLOOR.lock().map_err(|_| {
        RowFormatActivationError::Persist("format compatibility floor lock poisoned".to_string())
    })
}

fn store_installed_floor(
    floor: FormatCompatibilityFloor,
    path: PathBuf,
) -> std::result::Result<(), RowFormatActivationError> {
    let writer_enabled = floor.row_v2_writer_enabled;
    let mut guard = lock_floor()?;
    *guard = Some(InstalledFormatFloor { floor, path });
    ROW_V2_WRITERS_ENABLED.store(writer_enabled, std::sync::atomic::Ordering::Release);
    Ok(())
}

fn validate_floor(
    floor: &FormatCompatibilityFloor,
    writer_enabled: bool,
) -> std::result::Result<(), RowFormatActivationError> {
    let Some(agreed) = eligible_row_format(floor)? else {
        if writer_enabled {
            return Err(RowFormatActivationError::NoEligibleReplica(ROW_FORMAT_V2));
        }
        return Ok(());
    };
    if floor.min_supported_row_format > agreed {
        let replica = blocking_replica(floor, agreed).unwrap_or_else(|| "unknown".to_string());
        return Err(RowFormatActivationError::IncompatibleReplica {
            replica_id: replica,
            max_readable_row_format: agreed,
            required: floor.min_supported_row_format,
        });
    }
    if writer_enabled {
        if let Some((replica_id, max_readable_row_format)) = first_below(floor, ROW_FORMAT_V2) {
            return Err(RowFormatActivationError::IncompatibleReplica {
                replica_id,
                max_readable_row_format,
                required: ROW_FORMAT_V2,
            });
        }
    }
    Ok(())
}

fn eligible_row_format(
    floor: &FormatCompatibilityFloor,
) -> std::result::Result<Option<u8>, RowFormatActivationError> {
    let mut agreed: Option<u8> = None;
    for replica in floor.replicas.iter().filter(|replica| replica.eligible) {
        if replica.replica_id.is_empty() {
            return Err(RowFormatActivationError::Persist(
                "eligible replica id is empty".to_string(),
            ));
        }
        agreed = Some(agreed.map_or(replica.max_readable_row_format, |current| {
            current.min(replica.max_readable_row_format)
        }));
    }
    Ok(agreed)
}

fn first_below(floor: &FormatCompatibilityFloor, required: u8) -> Option<(String, u8)> {
    floor.replicas.iter().find_map(|replica| {
        if replica.eligible && replica.max_readable_row_format < required {
            Some((replica.replica_id.clone(), replica.max_readable_row_format))
        } else {
            None
        }
    })
}

fn blocking_replica(floor: &FormatCompatibilityFloor, agreed: u8) -> Option<String> {
    floor.replicas.iter().find_map(|replica| {
        if replica.eligible && replica.max_readable_row_format == agreed {
            Some(replica.replica_id.clone())
        } else {
            None
        }
    })
}

fn persist_floor(
    floor: &FormatCompatibilityFloor,
    path: &Path,
) -> std::result::Result<(), RowFormatActivationError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|err| RowFormatActivationError::Persist(err.to_string()))?;
    let bytes = serde_json::to_vec(floor)
        .map_err(|err| RowFormatActivationError::Persist(err.to_string()))?;
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("format-floor.json");
    let tmp = parent.join(format!(".{file_name}.tmp"));
    fs::write(&tmp, bytes).map_err(|err| RowFormatActivationError::Persist(err.to_string()))?;
    fs::rename(&tmp, path).map_err(|err| RowFormatActivationError::Persist(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) struct RowFormatTestGuard {
    _lock: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl RowFormatTestGuard {
    pub(crate) fn enter() -> Self {
        let lock = ROW_FORMAT_TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        ROW_FORMAT_TEST_LOCK_HELD.with(|held| held.set(true));
        ROW_V2_WRITERS_ENABLED.store(false, std::sync::atomic::Ordering::Release);
        if let Ok(mut slot) = FORMAT_FLOOR.lock() {
            *slot = None;
        }
        Self { _lock: lock }
    }
}

#[cfg(test)]
impl Drop for RowFormatTestGuard {
    fn drop(&mut self) {
        ROW_V2_WRITERS_ENABLED.store(false, std::sync::atomic::Ordering::Release);
        if let Ok(mut slot) = FORMAT_FLOOR.lock() {
            *slot = None;
        }
        ROW_FORMAT_TEST_LOCK_HELD.with(|held| held.set(false));
    }
}

#[cfg(test)]
pub(crate) fn row_format_test_lock_held_by_this_thread() -> bool {
    ROW_FORMAT_TEST_LOCK_HELD.with(|held| held.get())
}

#[cfg(test)]
pub(crate) fn acquire_row_format_encode_lock() -> MutexGuard<'static, ()> {
    ROW_FORMAT_TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
pub(crate) fn clear_installed_format_floor_memory() {
    ROW_V2_WRITERS_ENABLED.store(false, std::sync::atomic::Ordering::Release);
    if let Ok(mut slot) = FORMAT_FLOOR.lock() {
        *slot = None;
    }
}

#[inline]
pub(crate) fn begin_envelope(
    object_kind: ObjectKind,
    schema_version: u16,
    flags: u16,
    estimated_payload_len: usize,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + estimated_payload_len);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    bytes.push(object_kind as u8);
    bytes.extend_from_slice(&schema_version.to_le_bytes());
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 4]);
    bytes
}

#[inline]
pub(crate) fn finish_envelope(mut bytes: Vec<u8>) -> Result<EncodedObject> {
    let payload_len = u32::try_from(bytes.len() - HEADER_LEN)
        .map_err(|_| SerializationError::Encode("payload exceeds u32 length".to_string()))?;
    bytes[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&payload_len.to_le_bytes());
    Ok(EncodedObject::from_bytes(bytes))
}

pub(crate) fn encode_envelope(
    object_kind: ObjectKind,
    schema_version: u16,
    payload: &[u8],
) -> Result<EncodedObject> {
    encode_envelope_with_flags(object_kind, schema_version, 0, payload)
}

pub(crate) fn encode_envelope_with_flags(
    object_kind: ObjectKind,
    schema_version: u16,
    flags: u16,
    payload: &[u8],
) -> Result<EncodedObject> {
    let mut bytes = begin_envelope(object_kind, schema_version, flags, payload.len());
    bytes.extend_from_slice(payload);
    finish_envelope(bytes)
}

pub(crate) fn decode_envelope(
    bytes: &[u8],
    expected_kind: ObjectKind,
) -> Result<(EnvelopeHeader, &[u8])> {
    if bytes.len() < HEADER_LEN {
        return Err(SerializationError::Truncated);
    }
    if bytes[0..4] != MAGIC {
        return Err(SerializationError::InvalidMagic);
    }
    let protocol_version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if protocol_version != PROTOCOL_VERSION {
        return Err(SerializationError::UnsupportedProtocolVersion {
            found: protocol_version,
            supported: PROTOCOL_VERSION,
        });
    }
    let object_kind = ObjectKind::from_u8(bytes[6])?;
    if object_kind != expected_kind {
        return Err(SerializationError::WrongObjectKind {
            expected: expected_kind,
            found: object_kind,
        });
    }
    let schema_version = u16::from_le_bytes([bytes[7], bytes[8]]);
    let flags = u16::from_le_bytes([bytes[9], bytes[10]]);
    let supported_mask = match object_kind {
        ObjectKind::Row => SUPPORTED_ROW_FLAGS,
        _ => 0,
    };
    if flags & !supported_mask != 0 {
        return Err(SerializationError::Decode(format!(
            "unsupported envelope flags 0x{flags:04x} for {object_kind:?} (supported mask \
             0x{supported_mask:04x})"
        )));
    }
    let payload_len = u32::from_le_bytes([bytes[11], bytes[12], bytes[13], bytes[14]]) as usize;
    let payload_start = HEADER_LEN;
    let payload_end =
        payload_start.checked_add(payload_len).ok_or(SerializationError::Truncated)?;
    if bytes.len() < payload_end {
        return Err(SerializationError::Truncated);
    }
    if bytes.len() != payload_end {
        return Err(SerializationError::Decode(
            "trailing bytes after envelope payload".to_string(),
        ));
    }
    Ok((
        EnvelopeHeader {
            protocol_version,
            object_kind,
            schema_version,
            flags,
        },
        &bytes[payload_start..payload_end],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::PROTOCOL_VERSION;

    #[test]
    fn envelope_roundtrip() {
        let encoded = encode_envelope(ObjectKind::Generic, 3, &[1, 2, 3]).unwrap();
        let (header, payload) = decode_envelope(encoded.as_slice(), ObjectKind::Generic).unwrap();
        assert_eq!(header.protocol_version, PROTOCOL_VERSION);
        assert_eq!(header.object_kind, ObjectKind::Generic);
        assert_eq!(header.schema_version, 3);
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn rejects_wrong_magic() {
        let err = decode_envelope(b"NOPE", ObjectKind::Generic).unwrap_err();
        assert!(matches!(err, SerializationError::InvalidMagic | SerializationError::Truncated));
    }

    #[test]
    fn rejects_wrong_object_kind() {
        let encoded = encode_envelope(ObjectKind::Row, 1, &[]).unwrap();
        let err = decode_envelope(encoded.as_slice(), ObjectKind::Generic).unwrap_err();
        assert!(matches!(
            err,
            SerializationError::WrongObjectKind {
                expected: ObjectKind::Generic,
                found: ObjectKind::Row,
            }
        ));
    }

    #[test]
    fn rejects_future_protocol_version() {
        let mut bytes = encode_envelope(ObjectKind::Generic, 1, &[]).unwrap().into_bytes();
        bytes[4..6].copy_from_slice(&99u16.to_le_bytes());
        let err = decode_envelope(&bytes, ObjectKind::Generic).unwrap_err();
        assert!(matches!(
            err,
            SerializationError::UnsupportedProtocolVersion {
                found: 99,
                supported: PROTOCOL_VERSION,
            }
        ));
    }

    #[test]
    fn rejects_truncated_payload() {
        let mut bytes = encode_envelope(ObjectKind::Generic, 1, &[9, 9]).unwrap().into_bytes();
        bytes.pop();
        let err = decode_envelope(&bytes, ObjectKind::Generic).unwrap_err();
        assert_eq!(err, SerializationError::Truncated);
    }

    fn floor_path(label: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("kalam-row-format-{label}-{unique}.json"))
    }

    fn replica(id: &str, eligible: bool, max_readable: u8) -> ReplicaFormatCapability {
        ReplicaFormatCapability {
            replica_id: id.to_string(),
            eligible,
            max_readable_row_format: max_readable,
        }
    }

    #[cfg(feature = "backend-adapters")]
    fn sample_user_row() -> (kalamdb_commons::models::rows::UserTableRow, crate::row::StorageSchema)
    {
        use std::collections::BTreeMap;

        use crate::row::{StorageDataType, StorageField, StorageSchema};
        use datafusion_common::ScalarValue;
        use kalamdb_commons::{
            ids::VersionId,
            models::{rows::Row, UserId},
        };

        let schema = StorageSchema::new(1, vec![StorageField::new("id", StorageDataType::Int64)]);
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(7)));
        let row = kalamdb_commons::models::rows::UserTableRow {
            user_id: UserId::new("user-a"),
            _version: VersionId::try_from_i64(3).expect("version"),
            _deleted: false,
            fields: Row { values },
        };
        (row, schema)
    }

    #[cfg(feature = "backend-adapters")]
    #[test]
    fn mixed_row_formats_stay_readable() {
        use crate::row::{decode_user_row, encode_user_row, encode_user_row_v2};
        use kalamdb_commons::models::UserId;

        let _guard = RowFormatTestGuard::enter();
        let (row, schema) = sample_user_row();
        let v1 = encode_user_row(&row, &schema).expect("v1 encode");
        let v2 = encode_user_row_v2(&row, &schema).expect("v2 encode");
        assert_eq!(u16::from_le_bytes([v1.as_slice()[9], v1.as_slice()[10]]) & FLAG_ROW_BODY_V2, 0);
        assert_ne!(u16::from_le_bytes([v2.as_slice()[9], v2.as_slice()[10]]) & FLAG_ROW_BODY_V2, 0);
        let user_id = UserId::new("user-a");
        let decoded_v1 =
            decode_user_row(v1.as_slice(), &schema, user_id.clone(), row._version).expect("v1");
        let decoded_v2 =
            decode_user_row(v2.as_slice(), &schema, user_id, row._version).expect("v2");
        assert_eq!(decoded_v1, row);
        assert_eq!(decoded_v2, row);
    }

    #[test]
    fn incompatible_eligible_replica_blocks_row_v2_writer() {
        let _guard = RowFormatTestGuard::enter();
        let path = floor_path("blocked");
        let floor = FormatCompatibilityFloor {
            min_supported_row_format: ROW_FORMAT_V1,
            row_v2_writer_enabled: false,
            secondary_index_format_version: 1,
            replicas: vec![
                replica("replica-old", true, ROW_FORMAT_V1),
                replica("replica-draining", false, ROW_FORMAT_V2),
            ],
        };
        install_format_compatibility_floor(floor, &path).expect("install");

        let err = set_row_v2_writer_enabled(true).expect_err("blocked");
        assert!(matches!(
            err,
            RowFormatActivationError::IncompatibleReplica { ref replica_id, .. }
                if replica_id == "replica-old"
        ));
        assert!(!is_row_v2_writer_enabled());

        clear_installed_format_floor_memory();
        let loaded = load_format_compatibility_floor(&path).expect("reload");
        assert!(!loaded.row_v2_writer_enabled);
        assert!(set_row_v2_writer_enabled(true).is_err());
        assert!(!is_row_v2_writer_enabled());
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(feature = "backend-adapters")]
    #[test]
    fn disabling_row_v2_writer_leaves_existing_rows_readable() {
        use crate::row::{decode_user_row, encode_user_row};
        use kalamdb_commons::models::UserId;

        let _guard = RowFormatTestGuard::enter();
        let path = floor_path("disable");
        let floor = FormatCompatibilityFloor {
            min_supported_row_format: ROW_FORMAT_V1,
            row_v2_writer_enabled: false,
            secondary_index_format_version: 3,
            replicas: vec![
                replica("replica-a", true, ROW_FORMAT_V2),
                replica("replica-offline", false, ROW_FORMAT_V1),
            ],
        };
        install_format_compatibility_floor(floor, &path).expect("install");
        let (row, schema) = sample_user_row();
        let v1 = encode_user_row(&row, &schema).expect("v1");

        set_row_v2_writer_enabled(true).expect("enable");
        assert!(is_row_v2_writer_enabled());
        let installed = format_compatibility_floor().expect("floor").expect("installed");
        assert_eq!(installed.secondary_index_format_version, 3);
        assert_eq!(installed.min_supported_row_format, ROW_FORMAT_V2);
        let v2 = encode_user_row(&row, &schema).expect("v2");
        assert_ne!(u16::from_le_bytes([v2.as_slice()[9], v2.as_slice()[10]]) & FLAG_ROW_BODY_V2, 0);

        set_row_v2_writer_enabled(false).expect("disable");
        assert!(!is_row_v2_writer_enabled());
        let user_id = UserId::new("user-a");
        assert_eq!(
            decode_user_row(v1.as_slice(), &schema, user_id.clone(), row._version)
                .expect("decode v1"),
            row
        );
        assert_eq!(
            decode_user_row(v2.as_slice(), &schema, user_id, row._version).expect("decode v2"),
            row
        );
        let after_disable = encode_user_row(&row, &schema).expect("v1 again");
        assert_eq!(
            u16::from_le_bytes([after_disable.as_slice()[9], after_disable.as_slice()[10]])
                & FLAG_ROW_BODY_V2,
            0
        );

        clear_installed_format_floor_memory();
        let loaded = load_format_compatibility_floor(&path).expect("reload");
        assert!(!loaded.row_v2_writer_enabled);
        assert_eq!(loaded.secondary_index_format_version, 3);
        assert!(!is_row_v2_writer_enabled());
        let _ = std::fs::remove_file(&path);
    }
}
