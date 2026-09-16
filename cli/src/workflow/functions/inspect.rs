use crate::{
    error::Result,
    workflow::{
        sql::{build_workflow_client, execute_and_print},
        WorkflowContext,
    },
};

pub async fn show_function_status(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    let namespace = Some(env.namespace.as_str());
    output.status("active function module");
    execute_and_print(
        ctx,
        &client,
        "SELECT module_id, current_revision_id, runtime, abi_version, contract_hash FROM \
         system.modules ORDER BY module_id",
        namespace,
        "functions status",
    )
    .await?;
    output.status("procedures");
    execute_and_print(
        ctx,
        &client,
        "SELECT procedure_id, implementation, module_id, revision_id, security, signature, \
         return_type, grants FROM system.procedures ORDER BY procedure_id",
        namespace,
        "functions status",
    )
    .await?;
    output.status("function runtime");
    execute_and_print(
        ctx,
        &client,
        "SELECT metric_name, metric_value FROM system.stats WHERE metric_name LIKE \
         'function_memory%' OR metric_name LIKE 'function_instances%' ORDER BY metric_name",
        namespace,
        "functions status",
    )
    .await
}

pub async fn show_function_revisions(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    output.status("module revisions");
    execute_and_print(
        ctx,
        &client,
        "SELECT module_id, revision_id, is_current, contract_hash, artifact_bytes, created_at \
         FROM system.module_revisions ORDER BY created_at DESC",
        Some(env.namespace.as_str()),
        "functions revisions",
    )
    .await
}

pub async fn show_function_logs(ctx: &WorkflowContext, procedure: Option<&str>) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    output.status("procedure logs");
    let sql = match procedure {
        Some(name) => {
            let escaped = name.replace('\'', "''");
            format!(
                "SELECT timestamp, procedure_id, outcome, channel, level, error_code, message, \
                 duration_ms FROM system.procedure_logs WHERE procedure_id LIKE '%{escaped}%' \
                 ORDER BY timestamp DESC LIMIT 50"
            )
        },
        None => "SELECT timestamp, procedure_id, outcome, channel, level, error_code, message, \
                 duration_ms FROM system.procedure_logs ORDER BY timestamp DESC LIMIT 50"
            .to_string(),
    };
    execute_and_print(ctx, &client, &sql, Some(env.namespace.as_str()), "functions logs").await
}

pub async fn show_function_runtime(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &env)?;
    let namespace = Some(env.namespace.as_str());
    output.status("function memory");
    execute_and_print(
        ctx,
        &client,
        "SELECT metric_name, metric_value FROM system.stats WHERE metric_name LIKE \
         'function_memory%' OR metric_name LIKE 'function_instances%' ORDER BY metric_name",
        namespace,
        "functions runtime",
    )
    .await?;
    output.status("resident isolates");
    execute_and_print(
        ctx,
        &client,
        "SELECT instance_id, worker, module_id, revision_id, state, reserved_bytes, \
         used_heap_bytes, peak_heap_bytes, invocations FROM system.module_instances ORDER BY \
         worker, instance_id",
        namespace,
        "functions runtime",
    )
    .await?;
    output.status("in-flight root calls");
    execute_and_print(
        ctx,
        &client,
        "SELECT execution_id, request_id, procedure_id, revision_id, actor, origin, started_at, \
         depth FROM system.active_procedure_runs ORDER BY started_at",
        namespace,
        "functions runtime",
    )
    .await
}
