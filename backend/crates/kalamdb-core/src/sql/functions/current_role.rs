//! CURRENT_ROLE() function implementation
//!
//! Returns the current user's role from the session context.

use datafusion::error::DataFusionError;
use kalamdb_commons::Role;
use kalamdb_session_datafusion::SessionUserContext;

use super::scalar::{arity, scalar_udf, utf8_scalar, zero_arg_stable};

scalar_udf! {
    /// Returns the role of the current session user.
    pub struct CurrentRoleFunction;
    name = "kdb_current_role";
    aliases = ["current_role", "pg_catalog.current_role"];
    signature = zero_arg_stable();
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 0, "CURRENT_ROLE() takes no arguments")?;
        let session_ctx =
            args.config_options.extensions.get::<SessionUserContext>().ok_or_else(|| {
                DataFusionError::Execution(
                    "CURRENT_ROLE() failed: session user context not found".to_string(),
                )
            })?;
        let role_str = match session_ctx.role {
            Role::User => "user",
            Role::Service => "service",
            Role::Dba => "dba",
            Role::System => "system",
            Role::Anonymous => "anonymous",
        };
        Ok(utf8_scalar(role_str))
    }
}

#[cfg(test)]
mod tests {
    use datafusion::logical_expr::{ScalarUDF, ScalarUDFImpl};
    use kalamdb_commons::arrow_utils::ArrowDataType;

    use super::*;

    #[test]
    fn test_current_role_function_creation() {
        let func_impl = CurrentRoleFunction::new();
        let func = ScalarUDF::new_from_impl(func_impl);
        assert_eq!(func.name(), "kdb_current_role");
    }

    #[test]
    fn test_current_role_return_type() {
        let func_impl = CurrentRoleFunction::new();
        let return_type = func_impl.return_type(&[]);
        assert!(return_type.is_ok());
        assert_eq!(return_type.unwrap(), ArrowDataType::Utf8);
    }
}
