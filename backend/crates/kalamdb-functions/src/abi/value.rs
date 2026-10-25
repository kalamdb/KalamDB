//! Function-boundary values over DataFusion scalars and retained Arrow batches.

use std::sync::Arc;

use arrow::array::RecordBatch;
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
    pub type_id: Option<TypeId>,
    pub value: ScalarValue,
    pub json_sql: bool,
    pub named_fields: Option<Arc<[(String, RoutineValue)]>>,
    pub row_batches: Option<Arc<[RecordBatch]>>,
    pub read_view_batches: Option<Arc<[RecordBatch]>>,
    pub read_view_row: Option<usize>,
    pub host_named_input: bool,
}

impl RoutineValue {
    pub fn new(value: ScalarValue) -> Self {
        Self {
            type_id: None,
            value,
            json_sql: false,
            named_fields: None,
            row_batches: None,
            read_view_batches: None,
            read_view_row: None,
            host_named_input: false,
        }
    }

    /// Stored JSON text when this value is a JSON/JSONB document.
    pub fn json_sql_text(&self) -> Option<std::borrow::Cow<'_, str>> {
        if !self.json_sql {
            return None;
        }
        match &self.value {
            ScalarValue::Utf8(Some(text))
            | ScalarValue::LargeUtf8(Some(text))
            | ScalarValue::Utf8View(Some(text)) => Some(std::borrow::Cow::Borrowed(text)),
            ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Null => {
                Some(std::borrow::Cow::Borrowed("null"))
            },
            _ => None,
        }
    }

    pub fn json(value: ScalarValue) -> Self {
        let mut value = Self::new(value);
        value.json_sql = true;
        value
    }

    pub fn with_type_id(mut self, type_id: Option<TypeId>) -> Self {
        self.type_id = type_id;
        self
    }

    pub fn named_object(fields: Vec<(String, RoutineValue)>) -> Self {
        let mut value = Self::new(ScalarValue::Null);
        value.named_fields = Some(Arc::from(fields));
        value
    }

    pub fn from_row_batches(batches: Vec<RecordBatch>) -> Self {
        let mut value = Self::new(ScalarValue::Null);
        value.row_batches = Some(Arc::from(batches));
        value
    }

    pub fn from_read_view_batches(batches: Vec<RecordBatch>) -> Self {
        let mut value = Self::new(ScalarValue::Null);
        value.read_view_batches = Some(Arc::from(batches));
        value
    }

    pub fn from_read_view_row(batches: Arc<[RecordBatch]>, row_index: usize) -> Self {
        let mut value = Self::new(ScalarValue::Null);
        value.read_view_batches = Some(batches);
        value.read_view_row = Some(row_index);
        value
    }

    pub fn with_host_named_input(mut self, host_named_input: bool) -> Self {
        self.host_named_input = host_named_input;
        self
    }
}
