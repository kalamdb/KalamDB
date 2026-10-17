//! `pg_get_indexdef(index_oid [, column, pretty])` stub for PostgreSQL wire clients.

use kalamdb_commons::arrow_utils::ArrowDataType;

use super::scalar::{arity, scalar_udf, stable_exact, utf8_nulls};

scalar_udf! {
    pub struct PgGetIndexdefFunction;
    name = "pg_get_indexdef";
    aliases = ["pg_catalog.pg_get_indexdef"];
    signature = stable_exact(&[
        &[ArrowDataType::Int64],
        &[ArrowDataType::Int32],
        &[ArrowDataType::Int64, ArrowDataType::Int64, ArrowDataType::Boolean],
        &[ArrowDataType::Int64, ArrowDataType::Int32, ArrowDataType::Boolean],
        &[ArrowDataType::Int32, ArrowDataType::Int32, ArrowDataType::Boolean],
        &[ArrowDataType::Int64, ArrowDataType::UInt64, ArrowDataType::Boolean],
    ]);
    returns utf8;
    invoke(args) {
        arity(
            &args,
            |len| (1..=3).contains(&len),
            "pg_get_indexdef requires one or three arguments",
        )?;
        Ok(utf8_nulls(args.number_rows))
    }
}
