//! KDB_CURRENT_DATABASE() function implementation
//!
//! Returns the active catalog/database name for PostgreSQL client compatibility.

use super::scalar::{arity, scalar_udf, utf8_scalar, zero_arg_stable};

const DEFAULT_DATABASE_NAME: &str = "kalam";

scalar_udf! {
    /// Returns the current catalog name from the DataFusion session. PostgreSQL clients
    /// often query `CURRENT_DATABASE()` alongside `CURRENT_SCHEMA()` at connect time.
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
            DEFAULT_DATABASE_NAME
        } else {
            database
        };
        Ok(utf8_scalar(value))
    }
}
