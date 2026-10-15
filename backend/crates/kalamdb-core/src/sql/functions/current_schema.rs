//! KDB_CURRENT_SCHEMA() function implementation
//!
//! Returns the active namespace (PostgreSQL `current_schema`) from session config.

use kalamdb_commons::SchemaId;

use super::scalar::{arity, scalar_udf, utf8_scalar, zero_arg_stable};

scalar_udf! {
    /// Returns the current default schema/namespace from the DataFusion session catalog
    /// options. PostgreSQL clients call `CURRENT_SCHEMA()` during connection introspection.
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
        let value = if schema.is_empty() {
            SchemaId::default().as_str().to_string()
        } else {
            schema.to_string()
        };
        Ok(utf8_scalar(value.as_str()))
    }
}
