//! `col_description(oid, column_number)` stub for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::array::ArrayRef,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, TypeSignature, Volatility,
    },
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

/// Returns NULL column comments.
///
/// DBeaver selects `pg_catalog.col_description(...)` when loading column metadata.
/// KalamDB does not store per-column PostgreSQL comments yet.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct ColDescriptionFunction;

impl ColDescriptionFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for ColDescriptionFunction {
    fn name(&self) -> &str {
        "col_description"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| {
            Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![ArrowDataType::Int64, ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32, ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![ArrowDataType::Int64, ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32, ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int64, ArrowDataType::UInt64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32, ArrowDataType::UInt64]),
                ],
                Volatility::Stable,
            )
        })
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.len() != 2 {
            return Err(DataFusionError::Plan(
                "col_description(oid, column_number) requires two arguments".to_string(),
            ));
        }

        let mut builder = datafusion::arrow::array::StringBuilder::new();
        for _ in 0..args.number_rows {
            builder.append_null();
        }
        let array = builder.finish();
        Ok(ColumnarValue::Array(Arc::new(array) as ArrayRef))
    }
}
