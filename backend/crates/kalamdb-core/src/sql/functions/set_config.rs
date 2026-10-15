//! PostgreSQL set_config delegates to the shared transactional settings state.
use datafusion::{
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature},
};
use kalamdb_commons::arrow_utils::{arrow_utf8, ArrowDataType};
use kalamdb_session_datafusion::SessionConfigValues;

use super::scalar::{
    bool_at, cached_aliases, cached_signature, map_text, text_at, user_defined_volatile,
};
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SetConfigFunction;
impl SetConfigFunction {
    pub fn new() -> Self {
        Self
    }
}
impl ScalarUDFImpl for SetConfigFunction {
    fn name(&self) -> &str {
        "set_config"
    }
    fn aliases(&self) -> &[String] {
        cached_aliases!("pg_catalog.set_config")
    }
    fn signature(&self) -> &Signature {
        cached_signature!(user_defined_volatile())
    }
    fn coerce_types(&self, types: &[ArrowDataType]) -> DataFusionResult<Vec<ArrowDataType>> {
        if types.len() != 3 {
            return Err(DataFusionError::Plan("set_config requires text, text, boolean".into()));
        }
        Ok(vec![arrow_utf8(), arrow_utf8(), ArrowDataType::Boolean])
    }
    fn return_type(&self, _: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_utf8())
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        let settings =
            args.config_options.extensions.get::<SessionConfigValues>().ok_or_else(|| {
                DataFusionError::Execution("session settings are not installed".into())
            })?;
        map_text(&args, |row| {
            let Some(name) = text_at(&args.args[0], row)? else {
                return Ok(None);
            };
            settings
                .set_config(name, text_at(&args.args[1], row)?, bool_at(&args.args[2], row)?)
                .map(Some)
                .map_err(DataFusionError::Execution)
        })
    }
}
#[cfg(test)]
mod tests {
    use datafusion::{
        arrow::array::{Array, StringArray},
        logical_expr::ScalarUDF,
        prelude::SessionContext,
    };
    use kalamdb_session_datafusion::{install_extension, SessionConfigValues};

    use super::{super::current_setting::CurrentSettingFunction, *};

    #[tokio::test]
    async fn tabularis_saves_search_path_and_reads_it_back() {
        let ctx = session_with_config();
        let saved = scalar(
            &ctx,
            "SELECT set_config('app.tabularis_base_search_path', current_setting('search_path'), \
             false)",
        )
        .await;
        assert_eq!(saved, Some("\"$user\", public, default".to_string()));

        let restored =
            scalar(&ctx, "SELECT current_setting('app.tabularis_base_search_path')").await;
        assert_eq!(restored, Some("\"$user\", public, default".to_string()));
    }

    #[tokio::test]
    async fn set_config_overrides_search_path_for_current_setting() {
        let ctx = session_with_config();
        let updated = scalar(&ctx, "SELECT set_config('search_path', 'public', false)").await;
        assert_eq!(updated, Some("public".to_string()));
        assert_eq!(
            scalar(&ctx, "SELECT pg_catalog.current_setting('search_path')").await,
            Some("public".to_string())
        );
    }

    #[tokio::test]
    async fn untyped_placeholders_coerce() {
        let ctx = session_with_config();
        ctx.sql("SELECT set_config($1, $2, false)")
            .await
            .expect("untyped text placeholders must plan");
    }

    fn session_with_config() -> SessionContext {
        let ctx = SessionContext::new();
        ctx.register_udf(ScalarUDF::from(CurrentSettingFunction::new()));
        ctx.register_udf(ScalarUDF::from(SetConfigFunction::new()));
        let mut state = ctx.state();
        install_extension(&mut state, SessionConfigValues::new());
        SessionContext::new_with_state(state)
    }

    async fn scalar(ctx: &SessionContext, sql: &str) -> Option<String> {
        let batches = ctx.sql(sql).await.expect(sql).collect().await.expect(sql);
        let array = batches[0].column(0).as_any().downcast_ref::<StringArray>().expect("text");
        if array.is_null(0) {
            None
        } else {
            Some(array.value(0).to_string())
        }
    }
}
