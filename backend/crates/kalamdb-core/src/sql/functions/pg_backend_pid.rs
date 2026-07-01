//! `pg_backend_pid()` stub for PostgreSQL wire client introspection.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, Int32Array},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility},
};
use kalamdb_commons::arrow_utils::ArrowDataType;

/// Returns a stable backend PID for the current session.
///
/// PostgreSQL clients call `SELECT pg_backend_pid()` during connection setup.
/// We return `1` until per-session PIDs are wired through the execution context.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct PgBackendPidFunction;

impl PgBackendPidFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for PgBackendPidFunction {
    fn name(&self) -> &str {
        "pg_backend_pid"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| Signature::exact(vec![], Volatility::Stable))
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(ArrowDataType::Int32)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if !args.args.is_empty() {
            return Err(DataFusionError::Plan("pg_backend_pid() takes no arguments".to_string()));
        }

        let array = Int32Array::from(vec![1]);
        Ok(ColumnarValue::Array(Arc::new(array) as ArrayRef))
    }
}
