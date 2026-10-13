//! KalamDB SQL dialect.
//!
//! `KalamDbDialect` delegates to PostgreSQL. `preserve_type_id` keeps
//! `dialect.is::<PostgreSqlDialect>()` true so DataFusion and sqlparser treat
//! it as PostgreSQL. `Dialect::parse_statement` stays the inherited PostgreSQL
//! implementation, which returns `None` and lets the token cursor classify
//! Kalam-only forms.

use sqlparser::{derive_dialect, dialect::PostgreSqlDialect};

derive_dialect!(KalamDbDialect, PostgreSqlDialect, preserve_type_id = true);
