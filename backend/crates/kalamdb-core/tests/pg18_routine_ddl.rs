//! Integration tests for PostgreSQL 18 routine DDL and type dependency checks.

use crate::support;

use std::sync::Arc;

use kalamdb_commons::{
    models::{SchemaId, UserId},
    KalamDataType, Role,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};
use kalamdb_sql::ddl::{CreateProcedureStatement, CreateTypeBody, CreateTypeStatement};
use kalamdb_store::test_utils::TestDb;

fn app_schema() -> SchemaId {
    SchemaId::new("app")
}

async fn setup_app() -> (Arc<AppContext>, SqlExecutor, TestDb) {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app, test_db) = support::create_cluster_app_context_with_config(config).await;
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app), false);
    let executor = SqlExecutor::new(Arc::clone(&app), registry);
    (app, executor, test_db)
}

fn root_ctx(app: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(UserId::new("u_pg18_routine_ddl"), Role::Dba, app.base_session_context())
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> ExecutionResult {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql)
}

async fn exec_err(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> String {
    let ctx = root_ctx(app);
    executor
        .execute(sql, &ctx, vec![])
        .await
        .expect_err(&format!("expected error for `{sql}`"))
        .to_string()
}

#[test]
fn function_binding_discards_parenthesized_modifiers() {
    let wide = CreateProcedureStatement::parse(
        "CREATE PROCEDURE app.wide(amount numeric(12,2))",
        &app_schema(),
    )
    .expect("wide");
    let narrow = CreateProcedureStatement::parse(
        "CREATE PROCEDURE app.narrow(amount numeric(4,1))",
        &app_schema(),
    )
    .expect("narrow");

    let wide_arg = &wide.parameters[0].type_ref;
    let narrow_arg = &narrow.parameters[0].type_ref;
    assert_ne!(wide_arg.syntactic_modifier, narrow_arg.syntactic_modifier);

    let wide_signature = wide_arg.function_signature_type();
    let narrow_signature = narrow_arg.function_signature_type();
    assert_eq!(wide_signature, narrow_signature);
    assert!(wide_signature.syntactic_modifier.is_none());
    assert_eq!(wide_signature.name, "numeric");
    assert_eq!(wide_signature.data_type, KalamDataType::from_sql_name("numeric"));
}

#[test]
fn varchar_modifier_is_discarded_for_the_signature_only() {
    let statement = CreateProcedureStatement::parse(
        "CREATE PROCEDURE app.label(name varchar(10))",
        &app_schema(),
    )
    .expect("procedure");
    let argument = &statement.parameters[0].type_ref;
    assert_eq!(argument.syntactic_modifier.as_deref(), Some("10"));

    let signature = argument.function_signature_type();
    assert!(signature.syntactic_modifier.is_none());
    assert_eq!(signature.data_type, Some(KalamDataType::Text));
}

#[test]
fn composite_attributes_do_not_use_the_function_signature_rule() {
    let statement = CreateTypeStatement::parse(
        "CREATE TYPE app.payment AS (amount numeric(12,2))",
        &app_schema(),
    )
    .expect("composite");
    let CreateTypeBody::Composite { fields } = statement.body else {
        panic!("expected composite");
    };
    let amount = &fields[0].type_ref;
    assert_eq!(amount.syntactic_modifier.as_deref(), Some("12,2"));
    assert_eq!(
        amount.data_type,
        Some(KalamDataType::Decimal {
            precision: 12,
            scale: 2,
        })
    );
    assert_ne!(amount, &amount.function_signature_type());
}

#[test]
fn procedure_returns_is_rejected_and_function_return_discards_modifier_for_signature() {
    let err = CreateProcedureStatement::parse(
        "CREATE PROCEDURE app.charge() RETURNS numeric(12,2)",
        &app_schema(),
    )
    .expect_err("CREATE PROCEDURE ... RETURNS must be rejected in PG18");
    assert!(
        err.contains("CREATE PROCEDURE cannot declare a RETURNS clause"),
        "unexpected error: {err}"
    );

    let statement = CreateProcedureStatement::parse(
        "CREATE FUNCTION app.charge() RETURNS numeric(12,2)",
        &app_schema(),
    )
    .expect("function");
    let returned = statement.return_type.expect("RETURNS stays on the function");
    assert_eq!(returned.syntactic_modifier.as_deref(), Some("12,2"));
    let signature = returned.function_signature_type();
    assert!(signature.syntactic_modifier.is_none());
    assert_eq!(signature.data_type, KalamDataType::from_sql_name("numeric"));
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn routine_ddl_lifecycle_alter_drop_comment_grant_and_type_dependency() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA r_ddl").await;
    exec(&app, &executor, "CREATE SCHEMA r_ddl_target").await;
    exec(&app, &executor, "CREATE TYPE r_ddl.status_t AS ENUM ('open', 'closed')").await;

    exec(
        &app,
        &executor,
        "CREATE FUNCTION r_ddl.compute_fee(base INT, status r_ddl.status_t DEFAULT 'open') \
         RETURNS INT LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE COST 25 SET search_path TO r_ddl \
         AS $$ SELECT base + 10 $$",
    )
    .await;

    // Duplicate without OR REPLACE is rejected
    let dup = exec_err(
        &app,
        &executor,
        "CREATE FUNCTION r_ddl.compute_fee(base INT) RETURNS INT LANGUAGE SQL AS $$ SELECT base $$",
    )
    .await;
    assert!(dup.to_ascii_lowercase().contains("already exists"), "{dup}");

    // Kind mismatch on OR REPLACE is rejected
    let kind_mismatch = exec_err(
        &app,
        &executor,
        "CREATE OR REPLACE PROCEDURE r_ddl.compute_fee(base INT) LANGUAGE SQL AS $$ SELECT base $$",
    )
    .await;
    assert!(
        kind_mismatch.to_ascii_lowercase().contains("cannot change routine kind"),
        "{kind_mismatch}"
    );

    // DROP TYPE is blocked while referenced by routine parameter
    let drop_type_err = exec_err(&app, &executor, "DROP TYPE r_ddl.status_t").await;
    assert!(drop_type_err.to_ascii_lowercase().contains("referenced by"), "{drop_type_err}");

    // COMMENT ON FUNCTION and GRANT/REVOKE EXECUTE ON FUNCTION
    exec(
        &app,
        &executor,
        "COMMENT ON FUNCTION r_ddl.compute_fee(INT, r_ddl.status_t) IS 'Fee calculator'",
    )
    .await;
    exec(
        &app,
        &executor,
        "GRANT EXECUTE ON FUNCTION r_ddl.compute_fee(INT, r_ddl.status_t) TO PUBLIC",
    )
    .await;
    exec(
        &app,
        &executor,
        "REVOKE EXECUTE ON FUNCTION r_ddl.compute_fee(INT, r_ddl.status_t) FROM PUBLIC",
    )
    .await;

    // ALTER FUNCTION options, rename, and set schema
    exec(
        &app,
        &executor,
        "ALTER FUNCTION r_ddl.compute_fee(INT, r_ddl.status_t) STABLE CALLED ON NULL INPUT COST \
         50 RESET search_path",
    )
    .await;
    exec(
        &app,
        &executor,
        "ALTER FUNCTION r_ddl.compute_fee(INT, r_ddl.status_t) RENAME TO calc_fee",
    )
    .await;
    exec(
        &app,
        &executor,
        "ALTER FUNCTION r_ddl.calc_fee(INT, r_ddl.status_t) SET SCHEMA r_ddl_target",
    )
    .await;

    // DROP PROCEDURE on a FUNCTION is rejected
    let wrong_drop = exec_err(&app, &executor, "DROP PROCEDURE r_ddl_target.calc_fee").await;
    assert!(wrong_drop.to_ascii_lowercase().contains("not a procedure"), "{wrong_drop}");

    // DROP FUNCTION succeeds and unblocks DROP TYPE
    exec(&app, &executor, "DROP FUNCTION r_ddl_target.calc_fee(INT, r_ddl.status_t)").await;
    exec(&app, &executor, "DROP TYPE r_ddl.status_t").await;
}
