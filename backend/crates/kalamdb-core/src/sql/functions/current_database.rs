//! KDB_CURRENT_DATABASE() function implementation
//!
//! Returns the active catalog/database name for PostgreSQL client compatibility.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, StringArray},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility},
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

const DEFAULT_DATABASE_NAME: &str = "kalam";

/// KDB_CURRENT_DATABASE() scalar function implementation
///
/// Returns the current catalog name from the DataFusion session. PostgreSQL clients
/// often query `CURRENT_DATABASE()` alongside `CURRENT_SCHEMA()` at connect time.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct CurrentDatabaseFunction;

impl CurrentDatabaseFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for CurrentDatabaseFunction {
    fn name(&self) -> &str {
        "kdb_current_database"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| Signature::exact(vec![], Volatility::Stable))
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if !args.args.is_empty() {
            return Err(DataFusionError::Plan(
                "KDB_CURRENT_DATABASE() takes no arguments".to_string(),
            ));
        }

        let database = args.config_options.catalog.default_catalog.trim();
        let value = if database.is_empty() {
            DEFAULT_DATABASE_NAME
        } else {
            database
        };

        let array = StringArray::from(vec![value]);
        Ok(ColumnarValue::Array(Arc::new(array) as ArrayRef))
    }
}
