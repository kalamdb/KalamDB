//! `kalam db` — migrations, reset, and development fixtures.

pub mod migration;
pub mod reset;
pub mod seed;

pub use reset::DbResetOptions;

use crate::{
    error::Result,
    workflow::{
        db::migration::{
            apply::apply_migrations_for_db_command,
            create::{create_migration, CreateMigrationOptions},
            repair::{repair_project_migration_mark_applied, retry_project_migration},
            seal_draft_migration,
            status::migration_status,
        },
        WorkflowContext,
    },
};

pub async fn migrate_database(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    apply_migrations_for_db_command(ctx, &output).await
}

pub async fn reset_database(ctx: &WorkflowContext, options: DbResetOptions) -> Result<()> {
    let output = ctx.output();
    reset::reset_managed_database(ctx, &output, options).await
}

pub async fn seed_database(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    seed::seed_database(ctx, &output, seed::SeedMode::Force).await
}

pub fn create_project_migration(ctx: &WorkflowContext, name: String) -> Result<()> {
    let output = ctx.output();
    create_migration(&ctx.project_root, &ctx.config, &CreateMigrationOptions { name }, &output)
}

pub async fn show_migration_status(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    migration_status(ctx, &output).await
}

pub fn seal_project_migration(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    match seal_draft_migration(&ctx.project_root, &ctx.config, &output)? {
        Some(_) => Ok(()),
        None => {
            output.status("no draft migration to seal");
            Ok(())
        },
    }
}

pub async fn retry_failed_migration(ctx: &WorkflowContext, migration_id: String) -> Result<()> {
    retry_project_migration(ctx, migration_id).await
}

pub async fn mark_migration_applied(ctx: &WorkflowContext, migration_id: String) -> Result<()> {
    repair_project_migration_mark_applied(ctx, migration_id).await
}
