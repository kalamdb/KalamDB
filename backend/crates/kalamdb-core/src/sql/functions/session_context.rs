//! Session identity functions shared by HTTP SQL and the PostgreSQL wire protocol.
//!
//! DataFusion has no `current_user`, `current_schema`, or `current_database`.
//! These read the Kalam session. `now()` and `current_timestamp()` stay DataFusion's
//! clock. [`kalamdb_commons::BuiltinDefault`] is the name list for column defaults.

use datafusion::{error::DataFusionError, logical_expr::ScalarFunctionArgs};
use kalamdb_commons::{Role, SchemaId};
use kalamdb_session_datafusion::SessionUserContext;

use super::scalar::{arity, scalar_udf, utf8_scalar, zero_arg_stable};

fn session_user<'a>(
    args: &'a ScalarFunctionArgs,
    function: &str,
) -> Result<&'a SessionUserContext, DataFusionError> {
    args.config_options.extensions.get::<SessionUserContext>().ok_or_else(|| {
        DataFusionError::Execution(format!("{function} failed: session user context not found"))
    })
}

scalar_udf! {
    /// Returns the user id of the current session.
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
        let session_ctx = session_user(&args, "KDB_CURRENT_USER()")?;
        Ok(utf8_scalar(session_ctx.user_id.as_str()))
    }
}

scalar_udf! {
    /// Returns the role of the current session user.
    pub struct CurrentRoleFunction;
    name = "kdb_current_role";
    aliases = ["current_role", "pg_catalog.current_role"];
    signature = zero_arg_stable();
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 0, "CURRENT_ROLE() takes no arguments")?;
        let session_ctx = session_user(&args, "CURRENT_ROLE()")?;
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

scalar_udf! {
    /// Returns the current default schema from the DataFusion session catalog.
    pub struct CurrentSchemaFunction;
    name = "kdb_current_schema";
    aliases = [
        "current_schema",
        "current_namespace",
        "pg_catalog.current_schema",
    ];
    signature = zero_arg_stable();
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 0, "KDB_CURRENT_SCHEMA() takes no arguments")?;
        let schema = args.config_options.catalog.default_schema.trim();
        let fallback;
        let value = if schema.is_empty() {
            fallback = SchemaId::default();
            fallback.as_str()
        } else {
            schema
        };
        Ok(utf8_scalar(value))
    }
}

scalar_udf! {
    /// Returns the current catalog name from the DataFusion session.
    pub struct CurrentDatabaseFunction;
    name = "kdb_current_database";
    aliases = [
        "current_database",
        "current_catalog",
        "pg_catalog.current_database",
    ];
    signature = zero_arg_stable();
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 0, "KDB_CURRENT_DATABASE() takes no arguments")?;
        let database = args.config_options.catalog.default_catalog.trim();
        let value = if database.is_empty() {
            kalamdb_commons::constants::LOGICAL_DATABASE_NAME
        } else {
            database
        };
        Ok(utf8_scalar(value))
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
