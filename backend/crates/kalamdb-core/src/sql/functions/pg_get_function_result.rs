//! `pg_get_function_result(oid)` stub for PostgreSQL JDBC `getFunctions`.

use super::scalar::{arity, one_integer_stable, scalar_udf, utf8_rows};

scalar_udf! {
    pub struct PgGetFunctionResultFunction;
    name = "pg_get_function_result";
    aliases = ["pg_catalog.pg_get_function_result"];
    signature = one_integer_stable();
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 1, "pg_get_function_result(oid) requires one argument")?;
        Ok(utf8_rows("void", args.number_rows))
    }
}
