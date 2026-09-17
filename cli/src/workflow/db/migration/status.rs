//! Migration status reporting.

use crate::{
    error::Result,
    output::WorkflowOutput,
    workflow::{
        migration::{
            apply::load_server_migration_state, list_migration_files, migration_filename,
            MigrationStatus,
        },
        sql::build_workflow_client,
        WorkflowContext,
    },
};

pub async fn migration_status(ctx: &WorkflowContext, output: &WorkflowOutput) -> Result<()> {
    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    let state = load_server_migration_state(&client, &environment.namespace).await?;
    let files = list_migration_files(&ctx.config.migrations_dir(&ctx.project_root))?;

    if files.is_empty() {
        output.status("no migration files found");
        return Ok(());
    }

    let mut applied = Vec::new();
    let mut pending = Vec::new();
    let mut failed = Vec::new();
    let mut applying = Vec::new();
    for path in files {
        let filename = migration_filename(&path);
        match state.record(&filename).map(|record| record.status) {
            Some(MigrationStatus::Applied) => applied.push(filename),
            Some(MigrationStatus::Failed) => failed.push(filename),
            Some(MigrationStatus::Applying) => applying.push(filename),
            Some(MigrationStatus::Draft) | None => pending.push(filename),
        }
    }
    emit_status_group(output, "Applied", &applied);
    emit_status_group(output, "Pending", &pending);
    emit_status_group(output, "Failed", &failed);
    emit_status_group(output, "Applying", &applying);

    Ok(())
}

fn emit_status_group(output: &WorkflowOutput, label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    output.status(format!("{label}:"));
    for item in items {
        output.status(format!("  {item}"));
    }
}
