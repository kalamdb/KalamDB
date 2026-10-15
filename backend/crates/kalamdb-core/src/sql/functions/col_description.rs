//! `col_description(oid, column_number)` stub for PostgreSQL wire clients.

use kalamdb_commons::arrow_utils::ArrowDataType;

use super::scalar::{arity, scalar_udf, stable_exact, utf8_nulls};

scalar_udf! {
    /// Returns NULL column comments.
    ///
    /// DBeaver selects `pg_catalog.col_description(...)` when loading column metadata.
    /// KalamDB does not store per-column PostgreSQL comments yet.
    pub struct ColDescriptionFunction;
    name = "col_description";
    aliases = ["pg_catalog.col_description"];
    signature = stable_exact(&[
        &[ArrowDataType::Int64, ArrowDataType::Int64],
        &[ArrowDataType::Int32, ArrowDataType::Int32],
        &[ArrowDataType::Int64, ArrowDataType::Int32],
        &[ArrowDataType::Int32, ArrowDataType::Int64],
        &[ArrowDataType::Int64, ArrowDataType::UInt64],
        &[ArrowDataType::Int32, ArrowDataType::UInt64],
    ]);
    returns utf8;
    invoke(args) {
        arity(
            &args,
            |len| len == 2,
            "col_description(oid, column_number) requires two arguments",
        )?;
        Ok(utf8_nulls(args.number_rows))
    }
}
