//! Linked together so CI does not link the full engine once per source file.

#[path = "support/mod.rs"]
mod support;

#[path = "pg18_domains.rs"]
mod pg18_domains;
#[path = "pg18_execution_slice.rs"]
mod pg18_execution_slice;
#[path = "pg18_extension_types.rs"]
mod pg18_extension_types;
#[path = "pg18_js_sql_calls.rs"]
mod pg18_js_sql_calls;
#[path = "pg18_migration_harness.rs"]
mod pg18_migration_harness;
#[path = "pg18_routine_binding.rs"]
mod pg18_routine_binding;
#[path = "pg18_routine_catalog.rs"]
mod pg18_routine_catalog;
#[path = "pg18_routine_context.rs"]
mod pg18_routine_context;
#[path = "pg18_routine_ddl.rs"]
mod pg18_routine_ddl;
#[path = "pg18_routines.rs"]
mod pg18_routines;
#[path = "pg18_schema_evolution.rs"]
mod pg18_schema_evolution;
#[path = "pg18_subscription_schema.rs"]
mod pg18_subscription_schema;
#[path = "pg18_transaction_cases.rs"]
mod pg18_transaction_cases;
#[path = "pg18_type_declarations.rs"]
mod pg18_type_declarations;
#[path = "pg18_type_semantics.rs"]
mod pg18_type_semantics;
#[path = "pg_catalog_shims.rs"]
mod pg_catalog_shims;
#[path = "pg_type_slice.rs"]
mod pg_type_slice;
