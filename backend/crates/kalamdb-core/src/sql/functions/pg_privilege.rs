//! Privilege and comment stubs for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::array::{ArrayRef, BooleanBuilder, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, TypeSignature, Volatility,
    },
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlwaysTruePrivilegeFunction {
    name: &'static str,
}

impl AlwaysTruePrivilegeFunction {
    pub fn has_function_privilege() -> Self {
        Self {
            name: "has_function_privilege",
        }
    }

    pub fn has_table_privilege() -> Self {
        Self {
            name: "has_table_privilege",
        }
    }

    pub fn has_schema_privilege() -> Self {
        Self {
            name: "has_schema_privilege",
        }
    }
}

impl ScalarUDFImpl for AlwaysTruePrivilegeFunction {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| Signature::new(TypeSignature::VariadicAny, Volatility::Stable))
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(ArrowDataType::Boolean)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.is_empty() {
            return Err(DataFusionError::Plan(format!(
                "{} requires at least one argument",
                self.name
            )));
        }

        let mut builder = BooleanBuilder::with_capacity(args.number_rows);
        for _ in 0..args.number_rows {
            builder.append_value(true);
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}

/// `obj_description(oid [, catalog])` returns NULL until comments are stored.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct ObjDescriptionFunction;

impl ObjDescriptionFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for ObjDescriptionFunction {
    fn name(&self) -> &str {
        "obj_description"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| {
            Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![ArrowDataType::Int64]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32]),
                    TypeSignature::Exact(vec![ArrowDataType::Int64, ArrowDataType::Utf8]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32, ArrowDataType::Utf8]),
                    TypeSignature::Exact(vec![ArrowDataType::Int64, ArrowDataType::Utf8View]),
                    TypeSignature::Exact(vec![ArrowDataType::Int32, ArrowDataType::Utf8View]),
                ],
                Volatility::Stable,
            )
        })
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.is_empty() || args.args.len() > 2 {
            return Err(DataFusionError::Plan(
                "obj_description requires one or two arguments".to_string(),
            ));
        }

        let mut builder = StringBuilder::new();
        for _ in 0..args.number_rows {
            builder.append_null();
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}
