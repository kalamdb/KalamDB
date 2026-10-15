//! `pg_get_expr(expr, relid)` stub for PostgreSQL wire clients.

use kalamdb_commons::arrow_utils::ArrowDataType;

use super::scalar::{arity, scalar_udf, stable_exact, utf8_nulls};

scalar_udf! {
    pub struct PgGetExprFunction;
    name = "pg_get_expr";
    aliases = ["pg_catalog.pg_get_expr"];
    signature = stable_exact(&[
        &[ArrowDataType::Utf8, ArrowDataType::Int64],
        &[ArrowDataType::Utf8, ArrowDataType::Int32],
        &[ArrowDataType::Utf8, ArrowDataType::Null],
        &[ArrowDataType::Null, ArrowDataType::Int64],
        &[ArrowDataType::Null, ArrowDataType::Int32],
        &[ArrowDataType::Null, ArrowDataType::Null],
    ]);
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 2, "pg_get_expr(expr, relid) requires two arguments")?;
        Ok(utf8_nulls(args.number_rows))
    }
}
