//! KDB_CURRENT_SCHEMA() function implementation
//!
//! Returns the active namespace (PostgreSQL `current_schema`) from session config.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, StringArray},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility},
};
use kalamdb_commons::{
    arrow_utils::{arrow_utf8, ArrowDataType},
    NamespaceId,
};

/// KDB_CURRENT_SCHEMA() scalar function implementation
///
/// Returns the current default schema/namespace from the DataFusion session catalog
/// options. PostgreSQL clients call `CURRENT_SCHEMA()` during connection introspection.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct CurrentSchemaFunction;

impl CurrentSchemaFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for CurrentSchemaFunction {
    fn name(&self) -> &str {
        "kdb_current_schema"
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
            return Err(DataFusionError::Plan("KDB_CURRENT_SCHEMA() takes no arguments".to_string()));
        }

        let schema = args.config_options.catalog.default_schema.trim();
        let value = if schema.is_empty() {
            NamespaceId::default().as_str().to_string()
        } else {
            schema.to_string()
        };

        let array = StringArray::from(vec![value.as_str()]);
        Ok(ColumnarValue::Array(Arc::new(array) as ArrayRef))
    }
}
