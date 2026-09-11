//! `pg_get_function_result(oid)` stub for PostgreSQL JDBC `getFunctions`.

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
pub struct PgGetFunctionResultFunction;

impl PgGetFunctionResultFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for PgGetFunctionResultFunction {
    fn name(&self) -> &str {
        "pg_get_function_result"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| {
            Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![ArrowDataType::UInt64]),
                ],
                Volatility::Stable,
            )
        })
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.len() != 1 {
            return Err(DataFusionError::Plan(
                "pg_get_function_result(oid) requires one argument".to_string(),
            ));
        }

        let mut builder = StringBuilder::new();
        for _ in 0..args.number_rows {
            builder.append_value("void");
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}
