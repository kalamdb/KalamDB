//! Database migration command helpers.

use crate::{
    output::WorkflowOutput,
    workflow::{
        migration::apply::apply_migrations_for_db_command as apply_workflow_migrations,
        WorkflowContext,
    },
    Result,
};

pub async fn apply_migrations_for_db_command(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
) -> Result<()> {
    apply_workflow_migrations(ctx, output).await
}
