//! Privilege and comment stubs for PostgreSQL wire clients.

use datafusion::{
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature},
};
use kalamdb_commons::arrow_utils::ArrowDataType;

use super::scalar::{
    arity, bool_rows, cached_signature, scalar_udf, stable_exact, utf8_nulls, variadic_any_stable,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlwaysTruePrivilegeFunction {
    name: &'static str,
    aliases: Vec<String>,
}

impl AlwaysTruePrivilegeFunction {
    pub fn has_function_privilege() -> Self {
        Self {
            name: "has_function_privilege",
            aliases: vec!["pg_catalog.has_function_privilege".to_string()],
        }
    }

    pub fn has_table_privilege() -> Self {
        Self {
            name: "has_table_privilege",
            aliases: vec!["pg_catalog.has_table_privilege".to_string()],
        }
    }

    pub fn has_schema_privilege() -> Self {
        Self {
            name: "has_schema_privilege",
            aliases: vec!["pg_catalog.has_schema_privilege".to_string()],
        }
    }
}

impl ScalarUDFImpl for AlwaysTruePrivilegeFunction {
    fn name(&self) -> &str {
        self.name
    }

    fn aliases(&self) -> &[String] {
        &self.aliases
    }

    fn signature(&self) -> &Signature {
        cached_signature!(variadic_any_stable())
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
        Ok(bool_rows(true, args.number_rows))
    }
}

scalar_udf! {
    /// `obj_description(oid [, catalog])` returns NULL until comments are stored.
    pub struct ObjDescriptionFunction;
    name = "obj_description";
    aliases = ["pg_catalog.obj_description"];
    signature = stable_exact(&[
        &[ArrowDataType::Int64],
        &[ArrowDataType::Int32],
        &[ArrowDataType::Int64, ArrowDataType::Utf8],
        &[ArrowDataType::Int32, ArrowDataType::Utf8],
        &[ArrowDataType::Int64, ArrowDataType::Utf8View],
        &[ArrowDataType::Int32, ArrowDataType::Utf8View],
    ]);
    returns utf8;
    invoke(args) {
        arity(
            &args,
            |len| (1..=2).contains(&len),
            "obj_description requires one or two arguments",
        )?;
        Ok(utf8_nulls(args.number_rows))
    }
}
