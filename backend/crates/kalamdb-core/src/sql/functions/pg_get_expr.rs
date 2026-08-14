//! `pg_get_expr(expr, relid)` stub for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, TypeSignature, Volatility,
    },
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct PgGetExprFunction;

impl PgGetExprFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for PgGetExprFunction {
    fn name(&self) -> &str {
        "pg_get_expr"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| {
            Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![ArrowDataType::Utf8, ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Utf8, ArrowDataType::Int32]),
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
                "pg_get_expr(expr, relid) requires two arguments".to_string(),
            ));
        }

        let mut builder = StringBuilder::new();
        for _ in 0..args.number_rows {
            builder.append_null();
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}
