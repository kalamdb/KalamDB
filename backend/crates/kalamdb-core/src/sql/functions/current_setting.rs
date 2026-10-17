//! Session settings backed by the same state as SET, SHOW and pg_settings.
use datafusion::{
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature},
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};
use kalamdb_session::postgres_settings::{canonical_name, default_value};
use kalamdb_session_datafusion::SessionConfigValues;

use super::scalar::{
    bool_at, cached_aliases, cached_signature, map_text, text_at, user_defined_volatile,
};

/// Column name for a PostgreSQL `SHOW` statement.
///
/// The value is read later from the shared session settings. Describe only
/// needs the result column name.
pub fn classify_postgres_show(sql: &str) -> Option<String> {
    let kalamdb_sql::PostgresSetting::Show { name } =
        kalamdb_sql::PostgresSetting::parse(sql).ok()??
    else {
        return None;
    };
    Some(name)
}
#[cfg(test)]
fn setting_value(name: &str) -> &'static str {
    default_value(&canonical_name(name)).unwrap_or("")
}

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
    fn aliases(&self) -> &[String] {
        cached_aliases!("pg_catalog.current_setting")
    }
    fn signature(&self) -> &Signature {
        cached_signature!(user_defined_volatile())
    }
    fn coerce_types(&self, types: &[ArrowDataType]) -> DataFusionResult<Vec<ArrowDataType>> {
        if !matches!(types.len(), 1 | 2) {
            return Err(DataFusionError::Plan(
                "current_setting requires text and optional boolean".into(),
            ));
        }
        let mut result = vec![arrow_utf8()];
        if types.len() == 2 {
            result.push(ArrowDataType::Boolean);
        }
        Ok(result)
    }
    fn return_type(&self, _: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        let settings = args.config_options.extensions.get::<SessionConfigValues>();
        map_text(&args, |row| {
            let Some(name) = text_at(&args.args[0], row)? else {
                return Ok(None);
            };
            let value = settings
                .and_then(|s| s.get(name))
                .or_else(|| default_value(&canonical_name(name)).map(str::to_owned));
            let missing_ok =
                args.args.get(1).map(|v| bool_at(v, row)).transpose()?.unwrap_or(false);
            if value.is_none() && !missing_ok {
                return Err(DataFusionError::Execution(format!(
                    "unrecognized configuration parameter \"{name}\""
                )));
            }
            Ok(value)
        })
    }
}
#[cfg(test)]
mod tests {
    use datafusion::{
        arrow::array::{Array, StringArray},
        logical_expr::ScalarUDF,
        prelude::SessionContext,
        scalar::ScalarValue,
    };

    use super::*;

    #[test]
    fn known_settings_have_values() {
        assert_eq!(setting_value("server_version_num"), "160000");
        assert_eq!(setting_value("SERVER_VERSION"), "16.0");
        assert_eq!(setting_value("transaction_isolation"), "read committed");
        assert_eq!(setting_value("max_index_keys"), "32");
        assert_eq!(setting_value("max_identifier_length"), "63");
        assert_eq!(setting_value("unknown_setting"), "");
    }

    #[test]
    fn show_transaction_isolation_level_matches_jdbc() {
        assert_eq!(
            classify_postgres_show("SHOW TRANSACTION ISOLATION LEVEL").as_deref(),
            Some("transaction_isolation")
        );
        assert_eq!(
            classify_postgres_show("SHOW transaction_isolation;").as_deref(),
            Some("transaction_isolation")
        );
    }

    #[test]
    fn datafusion_show_all_is_not_a_postgres_guc_show() {
        assert_eq!(classify_postgres_show("SHOW ALL"), None);
        assert_eq!(classify_postgres_show("SHOW COLUMNS FROM t"), None);
        assert_eq!(classify_postgres_show("SHOW datafusion.execution.batch_size"), None);
    }

    #[tokio::test]
    async fn two_argument_current_setting_reads_search_path() {
        assert_eq!(
            scalar_setting("SELECT current_setting('search_path', true)").await,
            Some("\"$user\", public, default".to_string())
        );
        assert_eq!(
            scalar_setting("SELECT pg_catalog.current_setting('search_path', false)").await,
            Some("\"$user\", public, default".to_string())
        );
        assert_eq!(
            scalar_setting("SELECT current_setting('server_version_num')").await,
            Some("160000".to_string())
        );
        assert_eq!(scalar_setting("SELECT current_setting(NULL, true)").await, None);
        assert_eq!(scalar_setting("SELECT current_setting('no_such_guc', true)").await, None);
    }

    #[tokio::test]
    async fn parameterized_missing_ok_call_binds_the_setting_name() {
        let ctx = SessionContext::new();
        ctx.register_udf(ScalarUDF::from(CurrentSettingFunction::new()));
        let frame = ctx
            .sql("SELECT current_setting($1, true)")
            .await
            .expect("untyped $1 plus boolean must plan");
        let frame = frame
            .with_param_values(vec![ScalarValue::Utf8(Some("search_path".to_string()))])
            .expect("bind search_path");
        let batches = frame.collect().await.expect("execute bound current_setting");
        let value = string_or_null(&batches);
        assert_eq!(value, Some("\"$user\", public, default".to_string()));
    }

    async fn scalar_setting(sql: &str) -> Option<String> {
        let ctx = SessionContext::new();
        ctx.register_udf(ScalarUDF::from(CurrentSettingFunction::new()));
        let batches = ctx.sql(sql).await.expect(sql).collect().await.expect(sql);
        string_or_null(&batches)
    }

    fn string_or_null(batches: &[datafusion::arrow::array::RecordBatch]) -> Option<String> {
        let array = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("text result");
        if array.is_null(0) {
            None
        } else {
            Some(array.value(0).to_string())
        }
    }
}
