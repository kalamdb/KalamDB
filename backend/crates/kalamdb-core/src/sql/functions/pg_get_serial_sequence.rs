//! `pg_get_serial_sequence(table, column)` for `serial_id` columns.
//!
//! Returns the sequence name `SETVAL` uses, or null when the column is not a `serial_id`
//! default. `bigserial` / `snowflake_id` stays null so inspectors do not reset that counter.

use datafusion::logical_expr::{Signature, TypeSignature, Volatility};

use super::scalar::{arity, map_text, scalar_udf, text_at};
use crate::sql::serial_sequence::serial_sequence_for_regclass;

scalar_udf! {
    pub struct PgGetSerialSequenceFunction;
    name = "pg_get_serial_sequence";
    aliases = ["pg_catalog.pg_get_serial_sequence"];
    signature = Signature::new(TypeSignature::Any(2), Volatility::Stable);
    returns utf8;
    invoke(args) {
        arity(
            &args,
            |count| count == 2,
            "pg_get_serial_sequence requires a table and a column",
        )?;
        let configured = args.config_options.catalog.default_schema.trim();
        let default_schema = if configured.is_empty() {
            None
        } else {
            Some(configured.to_string())
        };
        map_text(&args, |row| {
            let table = text_at(&args.args[0], row)?;
            let column = text_at(&args.args[1], row)?;
            let (Some(table), Some(column)) = (table, column) else {
                return Ok(None);
            };
            Ok(serial_sequence_for_regclass(default_schema.as_deref(), table, column))
        })
    }
}
