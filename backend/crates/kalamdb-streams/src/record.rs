use kalamdb_commons::{ids::StreamTableRowId, models::StreamTableRow};
use serde::{Deserialize, Serialize};

/// Log record stored in memory and used by the stream table store API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StreamLogRecord {
    Put {
        row_id: StreamTableRowId,
        row:    StreamTableRow,
    },
    /// Soft-delete of a prior put. `timestamp` is the original put's
    /// `_timestamp` so file windows can locate the same segment.
    Delete {
        row_id:    StreamTableRowId,
        timestamp: i64,
    },
}

/// On-disk stream log record. Put payloads are ordinal KOBJ row bytes.
///
/// `timestamp` is the row's `_timestamp` (UTC epoch millis). It is stored on
/// the frame because KOBJ stream payloads do not embed system identity fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PersistedStreamLogRecord {
    Put {
        row_id:    StreamTableRowId,
        timestamp: i64,
        payload:   Vec<u8>,
    },
    /// Soft-delete. `timestamp` is the original put's ingestion time.
    Delete {
        row_id:    StreamTableRowId,
        timestamp: i64,
    },
}
