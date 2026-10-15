//! KDB_CURRENT_USER() function implementation
//!
//! Returns the current user_id from the session context.

use datafusion::error::DataFusionError;
use kalamdb_session_datafusion::SessionUserContext;

use super::scalar::{arity, scalar_udf, utf8_scalar, zero_arg_stable};

scalar_udf! {
    /// Returns the user_id of the current session user.
    /// `CURRENT_USER_ID()` resolves to this function through a DataFusion alias.
    pub struct CurrentUserFunction;
    name = "kdb_current_user";
    aliases = [
        "current_user",
        "current_user_id",
        "session_user",
        "user",
        "pg_catalog.current_user",
    ];
    signature = zero_arg_stable();
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 0, "KDB_CURRENT_USER() takes no arguments")?;
        let session_ctx =
            args.config_options.extensions.get::<SessionUserContext>().ok_or_else(|| {
                DataFusionError::Execution(
                    "KDB_CURRENT_USER() failed: session user context not found".to_string(),
                )
            })?;
        Ok(utf8_scalar(session_ctx.user_id.as_str()))
    }
}
