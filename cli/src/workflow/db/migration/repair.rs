//! Retry and repair helpers for `kalam db migration`.

use super::{
    apply::{
        load_server_migration_state, save_server_migration_record, save_server_migration_records,
    },
    MigrationStatus,
};
use crate::{
    error::{CLIError, Result},
    workflow::{sql::build_workflow_client, WorkflowContext},
};

pub async fn retry_project_migration(ctx: &WorkflowContext, migration_id: String) -> Result<()> {
    let output = ctx.output();
    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    let mut state = load_server_migration_state(&client, &environment.namespace).await?;
    let record = state.record(&migration_id).cloned().ok_or_else(|| {
        CLIError::ConfigurationError(format!("migration record not found: {migration_id}"))
    })?;
    if record.status != MigrationStatus::Failed {
        return Err(CLIError::ConfigurationError(format!(
            "migration {migration_id} is not failed"
        )));
    }
    state.upsert_applying(
        &record.migration_id,
        &record.namespace,
        record.sql.as_deref().unwrap_or_default(),
        record.source.as_deref().unwrap_or(&record.migration_id),
    );
    save_server_migration_record(&client, state.record(&migration_id).unwrap(), true).await?;
    output.status(format!("queued migration {migration_id} for retry"));
    Ok(())
}

pub async fn repair_project_migration_mark_applied(
    ctx: &WorkflowContext,
    migration_id: String,
) -> Result<()> {
    let output = ctx.output();
    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    let mut state = load_server_migration_state(&client, &environment.namespace).await?;
    if state.records_for_migration_id(&migration_id).is_empty() {
        return Err(CLIError::ConfigurationError(format!(
            "migration record not found: {migration_id}"
        )));
    }
    state.mark_applied(&migration_id);
    let records: Vec<_> = state.records_for_migration_id(&migration_id);
    save_server_migration_records(&client, &records).await?;
    state = load_server_migration_state(&client, &environment.namespace).await?;
    if state.has_failed_migration_id(&migration_id) || !state.is_applied(&migration_id) {
        return Err(CLIError::ConfigurationError(format!(
            "migration {migration_id} is still not applied on the server"
        )));
    }
    output.status(format!("marked migration {migration_id} as applied"));
    Ok(())
}
