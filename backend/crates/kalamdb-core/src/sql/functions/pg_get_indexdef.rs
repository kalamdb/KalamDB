//! `pg_get_indexdef(index_oid [, column, pretty])` stub for PostgreSQL wire clients.

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
pub struct PgGetIndexdefFunction;

impl PgGetIndexdefFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for PgGetIndexdefFunction {
    fn name(&self) -> &str {
        "pg_get_indexdef"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| {
            Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![
                        ArrowDataType::Int64,
                        ArrowDataType::Int64,
                        ArrowDataType::Boolean,
                    ]),
                    TypeSignature::Exact(vec![
                        ArrowDataType::Int64,
                        ArrowDataType::Int32,
                        ArrowDataType::Boolean,
                    ]),
                    TypeSignature::Exact(vec![
                        ArrowDataType::Int32,
                        ArrowDataType::Int32,
                        ArrowDataType::Boolean,
                    ]),
                    TypeSignature::Exact(vec![
                        ArrowDataType::Int64,
                        ArrowDataType::UInt64,
                        ArrowDataType::Boolean,
                    ]),
                ],
                Volatility::Stable,
            )
        })
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.is_empty() || args.args.len() > 3 {
            return Err(DataFusionError::Plan(
                "pg_get_indexdef requires one or three arguments".to_string(),
            ));
        }

        let mut builder = StringBuilder::new();
        for _ in 0..args.number_rows {
            builder.append_null();
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}
