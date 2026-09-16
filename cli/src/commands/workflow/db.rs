use kalam_cli::{workflow::db, Result};

use super::context::workflow_context;
use crate::args::{Cli, DbArgs, DbCommand, MigrationCommand};

pub(super) async fn handle_db(cli: &Cli, args: &DbArgs) -> Result<()> {
    let ctx = workflow_context(cli, args.project_dir.as_deref(), None)?;

    match &args.command {
        DbCommand::Migrate(_) => db::migrate_database(&ctx).await,
        DbCommand::Reset(reset_args) => {
            db::reset_database(
                &ctx,
                db::DbResetOptions {
                    assume_yes: reset_args.yes,
                },
            )
            .await
        },
        DbCommand::Seed => db::seed_database(&ctx).await,
        DbCommand::Migration(migration) => handle_migration_command(&ctx, &migration.command).await,
    }
}

async fn handle_migration_command(
    ctx: &kalam_cli::workflow::WorkflowContext,
    command: &MigrationCommand,
) -> Result<()> {
    match command {
        MigrationCommand::Create(create_args) => {
            db::create_project_migration(ctx, create_args.name.clone())
        },
        MigrationCommand::Status(_) => db::show_migration_status(ctx).await,
        MigrationCommand::Seal(_) => db::seal_project_migration(ctx),
        MigrationCommand::Retry(retry_args) => {
            db::retry_failed_migration(ctx, retry_args.migration_id.clone()).await
        },
        MigrationCommand::Repair(repair_args) => {
            if repair_args.mark_applied {
                db::mark_migration_applied(ctx, repair_args.migration_id.clone()).await
            } else {
                Err(kalam_cli::CLIError::ConfigurationError(
                    "migration repair requires --mark-applied".into(),
                ))
            }
        },
    }
}
