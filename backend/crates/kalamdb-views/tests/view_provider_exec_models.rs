use datafusion::{
    datasource::TableProvider, execution::context::SessionContext, physical_plan::collect,
};
use kalamdb_commons::models::{ReadContext, Role, UserId};
use kalamdb_datafusion_sources::exec::DeferredBatchExec;
use kalamdb_session_datafusion::SessionUserContext;
use kalamdb_views::create_datatypes_provider;

fn total_rows(batches: &[datafusion::arrow::record_batch::RecordBatch]) -> usize {
    batches.iter().map(|batch| batch.num_rows()).sum()
}

#[tokio::test]
async fn datatypes_view_scan_uses_deferred_batch_exec_and_returns_rows() {
    let provider = create_datatypes_provider();
    let mut state = SessionContext::new().state();
    state.config_mut().options_mut().extensions.insert(SessionUserContext::new(
        UserId::new("datatype-reader"),
        Role::System,
        ReadContext::Internal,
    ));
    let ctx = SessionContext::new_with_state(state);
    let state = ctx.state();
    let plan = provider.scan(&state, None, &[], None).await.expect("build datatypes plan");

    assert!(plan.is::<DeferredBatchExec>());

    let batches = collect(plan, state.task_ctx()).await.expect("collect datatypes plan");
    assert!(total_rows(&batches) > 0);
}
