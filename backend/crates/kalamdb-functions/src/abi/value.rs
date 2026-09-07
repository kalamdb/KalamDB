//! Function-boundary values over DataFusion scalars.

use bytes::Bytes;
use datafusion_common::ScalarValue;
use kalamdb_commons::TypeId;

/// Thin wrapper around the shared Arrow/DataFusion value model.
///
/// JSON/JSONB values set [`RoutineValue::json_sql`] so the V8 ABI may use
/// `JSON.parse` / `JSON.stringify`. Other types convert field-by-field.
/// Nested/HTTP hops may attach a FlatBuffer [`RoutineValue::transfer`] so the
/// isolate decodes once instead of walking the Arrow field graph.
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

    pub fn with_type_id(mut self, type_id: TypeId) -> Self {
        self.type_id = Some(type_id);
        self
    }

    pub fn with_transfer(mut self, bytes: Bytes, contract_hash: impl Into<String>) -> Self {
        self.transfer = Some(bytes);
        self.contract_hash = Some(contract_hash.into());
        self
    }
}
