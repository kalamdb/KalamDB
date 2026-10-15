//! `pg_backend_pid()` stub for PostgreSQL wire client introspection.

use super::scalar::{arity, int32_scalar, scalar_udf, zero_arg_stable};

scalar_udf! {
    /// Returns a stable backend PID for the current session.
    ///
    /// PostgreSQL clients call `SELECT pg_backend_pid()` during connection setup.
    /// We return `1` until per-session PIDs are wired through the execution context.
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
