//! Function-boundary values over DataFusion scalars and retained Arrow batches.

use std::sync::Arc;

use arrow::array::RecordBatch;
use bytes::Bytes;
use datafusion_common::ScalarValue;
use kalamdb_commons::TypeId;

/// Thin wrapper around the shared Arrow/DataFusion value model.
///
/// JSON/JSONB values set [`RoutineValue::json_sql`] so the V8 boundary
/// `JSON.parse`s the stored UTF-8. Query results may retain their
/// [`RecordBatch`] owners in [`RoutineValue::row_batches`] or as a lazy
/// read-view in [`RoutineValue::read_view_batches`] so the V8 boundary can
/// materialize or project directly without intermediate per-cell
/// `ScalarValue` + one-element array allocations.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutineValue {
    pub type_id:           Option<TypeId>,
    pub value:             ScalarValue,
    pub json_sql:          bool,
    pub transfer:          Option<Bytes>,
    pub contract_hash:     Option<String>,
    pub named_fields:      Option<Arc<[(String, RoutineValue)]>>,
    pub row_batches:       Option<Arc<[RecordBatch]>>,
    pub read_view_batches: Option<Arc<[RecordBatch]>>,
    pub read_view_row:     Option<usize>,
    pub host_named_input:  bool,
}

impl RoutineValue {
    pub fn new(value: ScalarValue) -> Self {
        Self {
            type_id: None,
            value,
            json_sql: false,
            transfer: None,
            contract_hash: None,
            named_fields: None,
            row_batches: None,
            read_view_batches: None,
            read_view_row: None,
            host_named_input: false,
        }
    }

    pub fn json(value: ScalarValue) -> Self {
        Self {
            type_id: None,
            value,
            json_sql: true,
            transfer: None,
            contract_hash: None,
            named_fields: None,
            row_batches: None,
            read_view_batches: None,
            read_view_row: None,
            host_named_input: false,
        }
    }

    pub fn named_object(fields: Vec<(String, RoutineValue)>) -> Self {
        Self {
            type_id:           None,
            value:             ScalarValue::Null,
            json_sql:          false,
            transfer:          None,
            contract_hash:     None,
            named_fields:      Some(Arc::from(fields)),
            row_batches:       None,
            read_view_batches: None,
            read_view_row:     None,
            host_named_input:  false,
        }
    }

    pub fn from_row_batches(batches: Vec<RecordBatch>) -> Self {
        Self {
            type_id:           None,
            value:             ScalarValue::Null,
            json_sql:          false,
            transfer:          None,
            contract_hash:     None,
            named_fields:      None,
            row_batches:       Some(Arc::from(batches)),
            read_view_batches: None,
            read_view_row:     None,
            host_named_input:  false,
        }
    }

    pub fn from_read_view_batches(batches: Vec<RecordBatch>) -> Self {
        Self {
            type_id:           None,
            value:             ScalarValue::Null,
            json_sql:          false,
            transfer:          None,
            contract_hash:     None,
            named_fields:      None,
            row_batches:       None,
            read_view_batches: Some(Arc::from(batches)),
            read_view_row:     None,
            host_named_input:  false,
        }
    }

    pub fn from_read_view_row(batches: Arc<[RecordBatch]>, row_index: usize) -> Self {
        Self {
            type_id:           None,
            value:             ScalarValue::Null,
            json_sql:          false,
            transfer:          None,
            contract_hash:     None,
            named_fields:      None,
            row_batches:       None,
            read_view_batches: Some(batches),
            read_view_row:     Some(row_index),
            host_named_input:  false,
        }
    }

    pub fn with_host_named_input(mut self, host_named_input: bool) -> Self {
        self.host_named_input = host_named_input;
        self
    }

    pub fn with_transfer(mut self, bytes: Bytes, contract_hash: impl Into<String>) -> Self {
        self.transfer = Some(bytes);
        self.contract_hash = Some(contract_hash.into());
        self
    }
}
