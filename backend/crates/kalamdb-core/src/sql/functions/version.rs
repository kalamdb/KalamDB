//! `version()` replaces DataFusion's builtin on the shared session.
//!
//! DataFusion's own `version()` returns `Apache DataFusion …`. PostgreSQL
//! clients on HTTP and pgwire expect a `PostgreSQL` version string.

use super::scalar::{arity, scalar_udf, utf8_rows, zero_arg_stable};

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
