//! `version()` compatibility function for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility},
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

const POSTGRES_COMPAT_VERSION: &str = "PostgreSQL 9.6.0 compatible KalamDB";

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct VersionFunction;

impl VersionFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for VersionFunction {
    fn name(&self) -> &str {
        "version"
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
            return Err(DataFusionError::Plan("version() takes no arguments".to_string()));
        }

        let mut values =
            StringBuilder::with_capacity(args.number_rows, POSTGRES_COMPAT_VERSION.len());
        for _ in 0..args.number_rows {
            values.append_value(POSTGRES_COMPAT_VERSION);
        }
        Ok(ColumnarValue::Array(Arc::new(values.finish()) as ArrayRef))
    }
}
