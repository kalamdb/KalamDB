//! Integration tests for PostgreSQL 18 routine catalog projections (`pg_catalog.pg_proc`,
//! `information_schema.routines`, `information_schema.parameters`, and `system.procedures`).

mod support;

use std::sync::Arc;

use datafusion::arrow::array::{Array, BooleanArray, StringArray};
use kalamdb_commons::{models::UserId, Role};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};
use kalamdb_store::test_utils::TestDb;

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
    ExecutionContext::new(
        UserId::new("u_pg18_routine_catalog"),
        Role::Dba,
        app.base_session_context(),
    )
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> ExecutionResult {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql)
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn pg_proc_information_schema_and_system_procedures_project_routine_metadata() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA r_cat").await;
    exec(
        &app,
        &executor,
        "CREATE FUNCTION r_cat.calc_tax(IN amount INT DEFAULT 100, OUT tax INT) RETURNS INT \
         LANGUAGE SQL IMMUTABLE STRICT AS $$ SELECT amount / 10 $$",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE PROCEDURE r_cat.audit_log(msg TEXT, VARIADIC tags TEXT[]) LANGUAGE SQL AS $$ \
         SELECT 1 $$",
    )
    .await;

    // 1. pg_catalog.pg_proc projects prokind ('f' vs 'p')
    let pg_proc_res = exec(
        &app,
        &executor,
        "SELECT proname, prokind FROM pg_catalog.pg_proc WHERE proname IN ('calc_tax', \
         'audit_log') ORDER BY proname",
    )
    .await;
    let ExecutionResult::Rows { batches, .. } = pg_proc_res else {
        panic!("expected rows from pg_proc");
    };
    let names = batches[0].column(0).as_any().downcast_ref::<StringArray>().expect("proname");
    let kinds = batches[0].column(1).as_any().downcast_ref::<StringArray>().expect("prokind");
    assert_eq!(names.value(0), "audit_log");
    assert_eq!(kinds.value(0), "p");
    assert_eq!(names.value(1), "calc_tax");
    assert_eq!(kinds.value(1), "f");

    // 2. information_schema.routines projects routine_type and is_deterministic
    let info_routines_res = exec(
        &app,
        &executor,
        "SELECT routine_name, routine_type, is_deterministic FROM information_schema.routines \
         WHERE routine_schema = 'r_cat' ORDER BY routine_name",
    )
    .await;
    let ExecutionResult::Rows { batches, .. } = info_routines_res else {
        panic!("expected rows from information_schema.routines");
    };
    let r_names = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("routine_name");
    let r_types = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("routine_type");
    let r_det = batches[0]
        .column(2)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .expect("is_deterministic");
    assert_eq!(r_names.value(0), "audit_log");
    assert_eq!(r_types.value(0), "PROCEDURE");
    assert!(!r_det.value(0));
    assert_eq!(r_names.value(1), "calc_tax");
    assert_eq!(r_types.value(1), "FUNCTION");
    assert!(r_det.value(1));

    // 3. information_schema.parameters projects parameter_mode, parameter_default, and is_variadic
    let info_params_res = exec(
        &app,
        &executor,
        "SELECT parameter_name, parameter_mode, parameter_default, is_variadic FROM \
         information_schema.parameters WHERE specific_schema = 'r_cat' ORDER BY specific_name, \
         ordinal_position",
    )
    .await;
    let ExecutionResult::Rows { batches, .. } = info_params_res else {
        panic!("expected rows from information_schema.parameters");
    };
    let p_names = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("parameter_name");
    let p_modes = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("parameter_mode");
    let p_defaults = batches[0]
        .column(2)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("parameter_default");
    let p_variadic = batches[0]
        .column(3)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .expect("is_variadic");
    assert_eq!(p_names.value(0), "msg");
    assert_eq!(p_modes.value(0), "IN");
    assert!(!p_variadic.value(0));
    assert_eq!(p_names.value(1), "tags");
    assert_eq!(p_modes.value(1), "VARIADIC");
    assert!(p_variadic.value(1));
    assert_eq!(p_names.value(2), "amount");
    assert_eq!(p_modes.value(2), "IN");
    assert!(!p_defaults.is_null(2));
    assert_eq!(p_defaults.value(2), "100");
    assert_eq!(p_names.value(3), "tax");
    assert_eq!(p_modes.value(3), "OUT");

    // 4. system.procedures projects both routines
    let sys_procs_res = exec(
        &app,
        &executor,
        "SELECT procedure_id, implementation, language FROM system.procedures WHERE schema = \
         'r_cat' ORDER BY procedure_id",
    )
    .await;
    let ExecutionResult::Rows { batches, .. } = sys_procs_res else {
        panic!("expected rows from system.procedures");
    };
    let proc_ids = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("procedure_id");
    assert_eq!(proc_ids.value(0), "r_cat.audit_log");
    assert_eq!(proc_ids.value(1), "r_cat.calc_tax");
}
