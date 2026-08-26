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

/// One PostgreSQL `SHOW` variable result (`name` is the result-column / GUC name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresShowVariable {
    pub name:  String,
    pub value: &'static str,
}

/// Classify PostgreSQL `SHOW <guc>` / `SHOW TRANSACTION ISOLATION LEVEL`.
///
/// Returns `None` for DataFusion `SHOW ALL` / `SHOW COLUMNS` / `SHOW datafusion.*`
/// so those stay on the DataFusion meta-command path.
pub fn classify_postgres_show(sql: &str) -> Option<PostgresShowVariable> {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    if trimmed.len() < 5 || !trimmed[..4].eq_ignore_ascii_case("SHOW") {
        return None;
    }
    // Next char after SHOW must be whitespace (avoid matching SHOWFOO).
    if !trimmed.as_bytes().get(4).is_some_and(|byte| byte.is_ascii_whitespace()) {
        return None;
    }
    let rest = trimmed[4..].trim_start();
    if rest.is_empty() {
        return None;
    }
    let rest_lower = rest.to_ascii_lowercase();
    if rest_lower == "all" || rest_lower.starts_with("all ") {
        return None;
    }
    if rest_lower == "columns" || rest_lower.starts_with("columns ") {
        return None;
    }

    let name_sql = if rest_lower.starts_with("variable ") {
        rest["variable ".len()..].trim()
    } else {
        rest
    };
    let raw_name = unquote_show_ident(name_sql);
    if raw_name.is_empty() {
        return None;
    }
    let name = canonical_guc_name(&raw_name);
    if name.starts_with("datafusion.") {
        return None;
    }

    Some(PostgresShowVariable {
        name:  name.clone(),
        value: setting_value(&name),
    })
}

fn unquote_show_ident(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if (bytes[0] == b'"' && bytes[trimmed.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[trimmed.len() - 1] == b'\'')
        {
            return trimmed[1..trimmed.len() - 1].replace("\"\"", "\"");
        }
    }
    trimmed.to_string()
}

fn canonical_guc_name(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "transaction isolation level" => "transaction_isolation".to_string(),
        "default transaction isolation level" => "default_transaction_isolation".to_string(),
        "transaction read only" => "transaction_read_only".to_string(),
        "time zone" => "timezone".to_string(),
        other => other.to_string(),
    }
}

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

fn setting_value(name: &str) -> &'static str {
    match name.trim().to_ascii_lowercase().as_str() {
        "server_version_num" => SERVER_VERSION_NUM,
        "server_version" => SERVER_VERSION,
        "server_encoding" => "UTF8",
        "client_encoding" => "UTF8",
        "timezone" | "time zone" => "UTC",
        "search_path" => "\"$user\", public, default",
        "integer_datetimes" => "on",
        "standard_conforming_strings" => "on",
        "transaction_isolation" | "default_transaction_isolation" => "read committed",
        "transaction_read_only" => "off",
        "datestyle" => "ISO, MDY",
        "extra_float_digits" => "3",
        "application_name" => "",
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
        assert_eq!(setting_value("transaction_isolation"), "read committed");
        assert_eq!(setting_value("unknown_setting"), "");
    }

    #[test]
    fn show_transaction_isolation_level_matches_jdbc() {
        let shown = classify_postgres_show("SHOW TRANSACTION ISOLATION LEVEL")
            .expect("JDBC SHOW TRANSACTION ISOLATION LEVEL must be recognized");
        assert_eq!(shown.name, "transaction_isolation");
        assert_eq!(shown.value, "read committed");

        let guc = classify_postgres_show("SHOW transaction_isolation;")
            .expect("SHOW transaction_isolation must be recognized");
        assert_eq!(guc.name, "transaction_isolation");
        assert_eq!(guc.value, "read committed");
    }

    #[test]
    fn datafusion_show_all_is_not_a_postgres_guc_show() {
        assert_eq!(classify_postgres_show("SHOW ALL"), None);
        assert_eq!(classify_postgres_show("SHOW COLUMNS FROM t"), None);
        assert_eq!(
            classify_postgres_show("SHOW datafusion.execution.batch_size"),
            None
        );
    }
}
