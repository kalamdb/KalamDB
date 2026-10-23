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

mod pg_stubs;
mod scalar;
mod session_context;

pub mod current_setting;
pub mod file_functions;
pub mod format_type;
pub mod pg_get_function_text;
pub mod pg_get_serial_sequence;
pub mod pg_privilege;
pub mod set_config;
pub mod setval;
pub mod snowflake_id;
pub mod ulid;
pub mod uuid_v7;

pub use current_setting::{classify_postgres_show, CurrentSettingFunction};
pub use file_functions::FileStorageFunction;
pub use format_type::FormatTypeFunction;
pub use kalamdb_vector::CosineDistanceFunction;
pub use pg_get_function_text::PgGetFunctionTextFunction;
pub use pg_get_serial_sequence::PgGetSerialSequenceFunction;
pub use pg_privilege::{AlwaysTruePrivilegeFunction, ObjDescriptionFunction};
pub use pg_stubs::{
    ColDescriptionFunction, PgBackendPidFunction, PgGetExprFunction, PgGetFunctionResultFunction,
    PgGetIndexdefFunction, VersionFunction,
};
pub use session_context::{
    CurrentDatabaseFunction, CurrentRoleFunction, CurrentSchemaFunction, CurrentUserFunction,
};
pub use set_config::SetConfigFunction;
pub use setval::SetvalFunction;
pub use snowflake_id::SnowflakeIdFunction;
pub use ulid::UlidFunction;
pub use uuid_v7::UuidV7Function;
