//! Function-boundary values over DataFusion scalars.

use bytes::Bytes;
use datafusion_common::ScalarValue;
use kalamdb_commons::TypeId;

/// Thin wrapper around the shared Arrow/DataFusion value model.
///
/// JSON/JSONB values set [`RoutineValue::json_sql`] so the V8 boundary
/// `JSON.parse`s the stored UTF-8. Every other value is converted field by
/// field in [`crate::convert`]. [`RoutineValue::transfer`] is an optional
/// flexbuffer of the same value for callers that already hold one; the V8
/// boundary does not decode it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutineValue {
    pub type_id:       Option<TypeId>,
    pub value:         ScalarValue,
    pub json_sql:      bool,
    pub transfer:      Option<Bytes>,
    pub contract_hash: Option<String>,
}

impl RoutineValue {
    pub fn new(value: ScalarValue) -> Self {
        Self {
            type_id: None,
            value,
            json_sql: false,
            transfer: None,
            contract_hash: None,
        }
    }

    pub fn json(value: ScalarValue) -> Self {
        Self {
            type_id: None,
            value,
            json_sql: true,
            transfer: None,
            contract_hash: None,
        }
    }

    pub fn with_transfer(mut self, bytes: Bytes, contract_hash: impl Into<String>) -> Self {
        self.transfer = Some(bytes);
        self.contract_hash = Some(contract_hash.into());
        self
    }
}
