//! SQL functions module
//!
//! This module provides custom SQL functions for DataFusion, including:
//! - ID generation functions: SNOWFLAKE_ID(), UUID_V7(), ULID()
//! - Context functions: CURRENT_USER(), CURRENT_ROLE(), CURRENT_SCHEMA(), CURRENT_DATABASE()
//!
//! Note: Temporal functions NOW() and CURRENT_TIMESTAMP() are provided by
//! DataFusion's built-in function library and do not need custom implementations.
//!
//! All functions follow the DataFusion ScalarUDFImpl pattern and can be used
//! in DEFAULT clauses, SELECT projections, and WHERE predicates.

pub mod col_description;
pub mod current_database;
pub mod current_role;
pub mod current_schema;
pub mod current_user;
pub mod format_type;
pub mod pg_backend_pid;
pub mod pg_get_expr;
pub mod snowflake_id;
pub mod ulid;
pub mod uuid_v7;
pub mod version;

pub use col_description::ColDescriptionFunction;
pub use current_database::CurrentDatabaseFunction;
pub use current_role::CurrentRoleFunction;
pub use current_schema::CurrentSchemaFunction;
pub use current_user::CurrentUserFunction;
pub use format_type::FormatTypeFunction;
pub use kalamdb_vector::CosineDistanceFunction;
pub use pg_backend_pid::PgBackendPidFunction;
pub use pg_get_expr::PgGetExprFunction;
pub use snowflake_id::SnowflakeIdFunction;
pub use ulid::UlidFunction;
pub use uuid_v7::UuidV7Function;
pub use version::VersionFunction;
