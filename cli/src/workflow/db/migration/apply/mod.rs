//! Apply pending migrations with local migration tracking.

mod recovery;
mod recovery_prompt;
mod server_state;

pub(crate) use server_state::{
    load_server_migration_state, save_server_migration_record, save_server_migration_records,
};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        display_project_path,
        io::read_migration_file,
        migration::{
            create::seal_draft_migration, list_migration_files, markers, migration_filename,
            DRAFT_MIGRATION_FILE,
        },
        sql::{build_workflow_client, ensure_namespace_exists, execute_sql_batch},
        WorkflowContext,
    },
};

pub struct ApplyMigrationOptions {
    pub force:         bool,
    pub include_draft: bool,
}

impl ApplyMigrationOptions {
    pub fn dev_force() -> Self {
        Self {
            force:         true,
            include_draft: false,
        }
    }

    pub fn dev_watch() -> Self {
        Self {
            force:         false,
            include_draft: false,
        }
    }

    pub fn dev_confirmed_draft() -> Self {
        Self {
            force:         true,
            include_draft: true,
        }
    }

    pub fn db_migrate() -> Self {
        Self {
            force:         true,
            include_draft: false,
        }
    }
}

pub async fn apply_pending_migrations(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    options: &ApplyMigrationOptions,
) -> Result<()> {
    let migrations_dir = ctx.config.migrations_dir(&ctx.project_root);
    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    let mut state = load_server_migration_state(&client, &environment.namespace).await?;
    let files = list_migration_files(&migrations_dir)?;

    if files.is_empty() {
        let draft_pending =
            options.include_draft && recovery::draft_migration_has_sql(&migrations_dir)?;
        if !draft_pending {
            output.status("no migrations to apply");
            return Ok(());
        }
    }

    recovery::validate_applied_checksums(&state, &files)?;
    recovery::handle_stuck_applying(
        &mut state,
        &migrations_dir,
        options,
        output,
        &client,
        &environment.namespace,
    )
    .await?;
    recovery::handle_failed_records(
        &mut state,
        &migrations_dir,
        options,
        output,
        &client,
        &environment.namespace,
    )
    .await?;

    let mut pending = recovery::pending_migration_files(&state, files)?;
    let draft_pending = options.include_draft
        && pending.is_empty()
        && recovery::draft_migration_has_sql(&migrations_dir)?;
    if pending.is_empty() && !draft_pending {
        output.status("all migrations already applied");
        return Ok(());
    }

    recovery::report_pending_migrations(&pending, draft_pending, output, options);

    ensure_namespace_exists(&client, &environment.namespace, output).await?;

    if draft_pending {
        output.progress_detail(
            "schema",
            format!("sealing draft migration {DRAFT_MIGRATION_FILE} into numbered history"),
        );
        output.status(format!(
            "sealing draft migration {DRAFT_MIGRATION_FILE} into the next numbered migration"
        ));
        let sealed =
            seal_draft_migration(&ctx.project_root, &ctx.config, output)?.ok_or_else(|| {
                CLIError::ConfigurationError(format!(
                    "draft migration {DRAFT_MIGRATION_FILE} disappeared before apply"
                ))
            })?;
        output.progress_detail("schema", format!("sealed draft migration {sealed}"));
        pending.push(migrations_dir.join(sealed));
    }

    let mut applied_count = 0;
    for path in pending {
        let filename = migration_filename(&path);
        let sql = read_migration_file(Some(&ctx.project_root), &path)?;

        let up_section = markers::extract_up_section(&sql);
        let record_exists = state.record(&filename).is_some();
        state.upsert_applying(&filename, &environment.namespace, &up_section, &filename);
        save_server_migration_record(&client, state.record(&filename).unwrap(), record_exists)
            .await?;
        if up_section.contains("__KALAM_TEST_SCHEMA_FAIL__") {
            let error = CLIError::ConfigurationError("schema apply failed (test hook)".into());
            state.mark_failed(&filename, error.to_string());
            save_server_migration_record(&client, state.record(&filename).unwrap(), true).await?;
            return Err(error);
        }
        output.status(format!(
            "applying migration {}",
            display_project_path(&ctx.project_root, &path)
        ));
        let result = execute_sql_batch(
            &client,
            &up_section,
            Some(environment.namespace.as_str()),
            output,
            &filename,
        )
        .await;
        match result {
            Ok(executed) => {
                state.mark_applied(&filename);
                save_server_migration_record(&client, state.record(&filename).unwrap(), true)
                    .await?;
                applied_count += 1;
                output.status(format!("applied {filename} ({executed} statement(s))"));
            },
            Err(error) => {
                state.mark_failed(&filename, error.to_string());
                save_server_migration_record(&client, state.record(&filename).unwrap(), true)
                    .await?;
                return Err(error);
            },
        }
    }

    if applied_count == 0 {
        output.status("all migrations already applied");
    } else {
        output.status(format!("applied {applied_count} migration(s)"));
    }

    Ok(())
}

pub async fn apply_migrations_for_db_command(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
) -> Result<()> {
    apply_pending_migrations(ctx, output, &ApplyMigrationOptions::db_migrate()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_watch_options_leave_draft_for_dev_prompt_manager() {
        let options = ApplyMigrationOptions::dev_watch();

        assert!(!options.force);
        assert!(!options.include_draft);
    }

    #[test]
    fn dev_force_options_apply_numbered_migrations_without_draft_prompt() {
        let options = ApplyMigrationOptions::dev_force();

        assert!(options.force);
        assert!(!options.include_draft);
    }

    #[test]
    fn dev_confirmed_draft_options_apply_draft_without_prompting_again() {
        let options = ApplyMigrationOptions::dev_confirmed_draft();

        assert!(options.force);
        assert!(options.include_draft);
    }
}
