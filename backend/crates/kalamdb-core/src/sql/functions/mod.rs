//! SQL functions module
//!
//! This module provides custom SQL functions for DataFusion, including:
//! - ID generation functions: SNOWFLAKE_ID(), UUID_V7(), ULID()
//! - Context functions: CURRENT_USER(), CURRENT_ROLE(), CURRENT_SCHEMA(), CURRENT_DATABASE()
//!
//! These are DataFusion `ScalarUDF`s on the shared session, so HTTP SQL and
//! pgwire call the same functions. DataFusion already provides NOW() and
//! CURRENT_TIMESTAMP(). It does not provide current_user, current_schema,
//! current_database, current_setting, or set_config.
//!
//! All functions follow the DataFusion ScalarUDFImpl pattern and can be used
//! in DEFAULT clauses, SELECT projections, and WHERE predicates.
//! Stateless functions share that shell through `scalar_udf!`.

mod scalar;

pub mod col_description;
pub mod current_database;
pub mod current_role;
pub mod current_schema;
pub mod current_setting;
pub mod current_user;
pub mod file_functions;
pub mod format_type;
pub mod pg_backend_pid;
pub mod pg_get_expr;
pub mod pg_get_function_result;
pub mod pg_get_function_text;
pub mod pg_get_indexdef;
pub mod pg_privilege;
pub mod set_config;
pub mod snowflake_id;
pub mod ulid;
pub mod uuid_v7;
pub mod version;

pub use col_description::ColDescriptionFunction;
pub use current_database::CurrentDatabaseFunction;
pub use current_role::CurrentRoleFunction;
pub use current_schema::CurrentSchemaFunction;
pub use current_setting::{classify_postgres_show, CurrentSettingFunction};
pub use current_user::CurrentUserFunction;
pub use file_functions::FileStorageFunction;
pub use format_type::FormatTypeFunction;
pub use kalamdb_vector::CosineDistanceFunction;
pub use pg_backend_pid::PgBackendPidFunction;
pub use pg_get_expr::PgGetExprFunction;
pub use pg_get_function_result::PgGetFunctionResultFunction;
pub use pg_get_function_text::PgGetFunctionTextFunction;
pub use pg_get_indexdef::PgGetIndexdefFunction;
pub use pg_privilege::{AlwaysTruePrivilegeFunction, ObjDescriptionFunction};
pub use set_config::SetConfigFunction;
pub use snowflake_id::SnowflakeIdFunction;
pub use ulid::UlidFunction;
pub use uuid_v7::UuidV7Function;
pub use version::VersionFunction;
