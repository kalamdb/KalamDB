//! `current_setting(name)` stub for PostgreSQL wire clients.

use std::sync::Arc;

use datafusion::{
    arrow::array::{Array, ArrayRef, StringArray, StringBuilder},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility},
    scalar::ScalarValue,
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};

/// PostgreSQL-compatible server version number (16.0) for client feature checks.
///
/// Tabularis and similar clients branch on `server_version_num >= 110000` for
/// `pg_proc.prokind` vs legacy `proisagg` columns.
const SERVER_VERSION_NUM: &str = "160000";
const SERVER_VERSION: &str = "16.0";

/// `current_setting(name)` for PostgreSQL client introspection.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct CurrentSettingFunction;

impl CurrentSettingFunction {
    pub fn new() -> Self {
        Self
    }
}

impl ScalarUDFImpl for CurrentSettingFunction {
    fn name(&self) -> &str {
        "current_setting"
    }

    fn signature(&self) -> &Signature {
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| Signature::exact(vec![arrow_utf8()], Volatility::Stable))
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if args.args.len() != 1 {
            return Err(DataFusionError::Plan(
                "current_setting(name) requires one text argument".to_string(),
            ));
        }

        let names = utf8_values(&args.args[0], args.number_rows)?;
        let mut builder = StringBuilder::with_capacity(names.len(), 16);
        for name in names {
            builder.append_value(setting_value(&name));
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish()) as ArrayRef))
    }
}

fn setting_value(name: &str) -> &str {
    match name.trim().to_ascii_lowercase().as_str() {
        "server_version_num" => SERVER_VERSION_NUM,
        "server_version" => SERVER_VERSION,
        "server_encoding" => "UTF8",
        "client_encoding" => "UTF8",
        "timezone" | "time zone" => "UTC",
        "search_path" => "\"$user\", public, default",
        "integer_datetimes" => "on",
        "standard_conforming_strings" => "on",
        _ => "",
    }
}

fn utf8_values(value: &ColumnarValue, row_count: usize) -> DataFusionResult<Vec<String>> {
    match value {
        ColumnarValue::Scalar(ScalarValue::Utf8(Some(text)))
        | ColumnarValue::Scalar(ScalarValue::LargeUtf8(Some(text)))
        | ColumnarValue::Scalar(ScalarValue::Utf8View(Some(text))) => {
            Ok(vec![text.clone(); row_count])
        },
        ColumnarValue::Scalar(ScalarValue::Utf8(None))
        | ColumnarValue::Scalar(ScalarValue::LargeUtf8(None))
        | ColumnarValue::Scalar(ScalarValue::Utf8View(None)) => Ok(vec![String::new(); row_count]),
        ColumnarValue::Array(array) => {
            if let Some(array) = array.as_any().downcast_ref::<StringArray>() {
                return Ok((0..array.len())
                    .map(|index| {
                        if array.is_null(index) {
                            String::new()
                        } else {
                            array.value(index).to_string()
                        }
                    })
                    .collect());
            }
            Err(DataFusionError::Plan(
                "current_setting(name) requires a text argument".to_string(),
            ))
        },
        _ => Err(DataFusionError::Plan(
            "current_setting(name) requires a text argument".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_settings_have_values() {
        assert_eq!(setting_value("server_version_num"), "160000");
        assert_eq!(setting_value("SERVER_VERSION"), "16.0");
        assert_eq!(setting_value("unknown_setting"), "");
    }
}
