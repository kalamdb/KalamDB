//! Constant PostgreSQL functions used by wire clients during connection setup.
//!
//! These return a fixed value. Catalog-backed text (`pg_get_functiondef` and
//! the live `pg_get_function_result`) stays in `pg_get_function_text`.

use kalamdb_commons::arrow_utils::ArrowDataType;

use super::scalar::{
    arity, int32_scalar, one_integer_stable, scalar_udf, stable_exact, utf8_nulls, utf8_rows,
    zero_arg_stable,
};

/// `version()` replaces DataFusion's builtin on the shared session.
///
/// DataFusion's own `version()` returns `Apache DataFusion …`. This string is
/// the function result. `server_version` in `pg_settings` is a different value.
const POSTGRES_COMPAT_VERSION: &str = "PostgreSQL 9.6.0 compatible KalamDB";

scalar_udf! {
    pub struct VersionFunction;
    name = "version";
    aliases = ["pg_catalog.version"];
    signature = zero_arg_stable();
    returns utf8;
    invoke(args) {
        arity(&args, |len| len == 0, "version() takes no arguments")?;
        Ok(utf8_rows(POSTGRES_COMPAT_VERSION, args.number_rows))
    }
}

scalar_udf! {
    /// Returns a stable backend PID for the current session.
    ///
    /// PostgreSQL clients call `SELECT pg_backend_pid()` during connection setup.
    /// The value stays `1` until per-session PIDs are wired through the execution context.
    pub struct PgBackendPidFunction;
    name = "pg_backend_pid";
    aliases = ["pg_catalog.pg_backend_pid"];
    signature = zero_arg_stable();
    returns int32;
    invoke(args) {
        arity(&args, |len| len == 0, "pg_backend_pid() takes no arguments")?;
        Ok(int32_scalar(1))
    }
}

scalar_udf! {
    /// Empty-node `pg_get_function_result`. A live catalog replaces this registration.
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

scalar_udf! {
    /// Returns NULL column comments.
    ///
    /// DBeaver selects `pg_catalog.col_description(...)` when loading column metadata.
    /// `pg_description` is the catalog view. This function stays NULL.
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
