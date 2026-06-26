use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::SelectOption,
    workflow::{
        migration::MigrationRecord,
        prompts::{read_single_key_decision, run_workflow_modal_prompt, WorkflowModalPrompt},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedMigrationDecision {
    Retry,
    Skip,
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StuckApplyingDecision {
    Retry,
    MarkFailed,
    Abort,
}

pub fn prompt_failed_migration_recovery(
    output: &WorkflowOutput,
    record: &MigrationRecord,
) -> Result<FailedMigrationDecision> {
    run_workflow_modal_prompt(output, &FailedMigrationPrompt::new(record))
}

pub fn prompt_stuck_applying_recovery(
    output: &WorkflowOutput,
    record: &MigrationRecord,
) -> Result<StuckApplyingDecision> {
    run_workflow_modal_prompt(output, &StuckApplyingPrompt::new(record))
}

struct FailedMigrationPrompt {
    context_lines: Vec<String>,
}

impl FailedMigrationPrompt {
    fn new(record: &MigrationRecord) -> Self {
        let reason = record.error_message.as_deref().unwrap_or("unknown error");
        Self {
            context_lines: vec![
                format!("Migration: {}", record.migration_id),
                format!("Reason: {reason}"),
            ],
        }
    }
}

impl WorkflowModalPrompt for FailedMigrationPrompt {
    type Decision = FailedMigrationDecision;

    fn message(&self) -> &str {
        "Migration failed previously. What should we do?"
    }

    fn banner_title(&self) -> &str {
        "Migration recovery"
    }

    fn banner_subtitle(&self) -> Option<&str> {
        Some("Choose how to continue before kalam dev starts managed processes.")
    }

    fn context_lines(&self) -> &[String] {
        &self.context_lines
    }

    fn select_options(&self) -> Vec<SelectOption<'static>> {
        vec![
            SelectOption::described("Retry", "Run the migration SQL again"),
            SelectOption::described("Skip", "Mark the migration as applied without running SQL"),
            SelectOption::described("Abort", "Stop schema apply and keep the failed migration"),
        ]
    }

    fn default_index(&self) -> usize {
        if self
            .context_lines
            .iter()
            .any(|line| line.to_ascii_lowercase().contains("already exists"))
        {
            1
        } else {
            0
        }
    }

    fn decision_from_selected_index(&self, index: usize) -> Result<Self::Decision> {
        Ok(match index {
            0 => FailedMigrationDecision::Retry,
            1 => FailedMigrationDecision::Skip,
            _ => FailedMigrationDecision::Abort,
        })
    }

    fn read_noninteractive_decision(&self) -> Result<Self::Decision> {
        read_single_key_decision("migration recovery input", |key| match key {
            'r' | 'R' => Ok(FailedMigrationDecision::Retry),
            's' | 'S' => Ok(FailedMigrationDecision::Skip),
            _ => Ok(FailedMigrationDecision::Abort),
        })
    }
}

struct StuckApplyingPrompt {
    context_lines: Vec<String>,
}

impl StuckApplyingPrompt {
    fn new(record: &MigrationRecord) -> Self {
        Self {
            context_lines: vec![format!(
                "Migration {} was interrupted before it finished applying.",
                record.migration_id
            )],
        }
    }
}

impl WorkflowModalPrompt for StuckApplyingPrompt {
    type Decision = StuckApplyingDecision;

    fn message(&self) -> &str {
        "Migration was interrupted. What should we do?"
    }

    fn banner_title(&self) -> &str {
        "Migration recovery"
    }

    fn banner_subtitle(&self) -> Option<&str> {
        Some("Choose how to continue before kalam dev starts managed processes.")
    }

    fn context_lines(&self) -> &[String] {
        &self.context_lines
    }

    fn select_options(&self) -> Vec<SelectOption<'static>> {
        vec![
            SelectOption::described("Retry", "Continue applying this migration"),
            SelectOption::described("Mark failed", "Record the migration as failed and continue"),
            SelectOption::described("Abort", "Stop schema apply"),
        ]
    }

    fn default_index(&self) -> usize {
        0
    }

    fn decision_from_selected_index(&self, index: usize) -> Result<Self::Decision> {
        Ok(match index {
            0 => StuckApplyingDecision::Retry,
            1 => StuckApplyingDecision::MarkFailed,
            _ => StuckApplyingDecision::Abort,
        })
    }

    fn read_noninteractive_decision(&self) -> Result<Self::Decision> {
        read_single_key_decision("migration recovery input", |key| match key {
            'r' | 'R' => Ok(StuckApplyingDecision::Retry),
            'f' | 'F' => Ok(StuckApplyingDecision::MarkFailed),
            _ => Ok(StuckApplyingDecision::Abort),
        })
    }
}

pub fn migration_recovery_abort_error(migration_id: &str) -> CLIError {
    CLIError::MigrationRecoveryAborted(format!(
        "migration {migration_id} recovery aborted; rerun with --force to retry automatically"
    ))
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::NamespaceId;

    use super::*;
    use crate::workflow::migration::{MigrationRecord, MigrationStatus};

    fn sample_failed_record() -> MigrationRecord {
        MigrationRecord {
            migration_id: "0001_auto.sql".into(),
            namespace: NamespaceId::new("dev"),
            migration_key: Some("dev:0001_auto.sql".into()),
            name: "auto".into(),
            checksum: "abc".into(),
            status: MigrationStatus::Failed,
            started_at: None,
            finished_at: None,
            error_message: Some("table already exists".into()),
            sql: None,
            source: Some("0001_auto.sql".into()),
            kalam_version: None,
        }
    }

    #[test]
    fn failed_prompt_defaults_to_skip_for_already_exists_errors() {
        let prompt = FailedMigrationPrompt::new(&sample_failed_record());
        assert_eq!(prompt.default_index(), 1);
    }

    #[test]
    fn failed_prompt_defaults_to_retry_for_other_errors() {
        let mut record = sample_failed_record();
        record.error_message = Some("connection reset".into());
        let prompt = FailedMigrationPrompt::new(&record);
        assert_eq!(prompt.default_index(), 0);
    }

    #[test]
    fn failed_prompt_maps_retry_skip_abort() {
        let prompt = FailedMigrationPrompt::new(&sample_failed_record());
        assert_eq!(prompt.decision_from_selected_index(0).unwrap(), FailedMigrationDecision::Retry);
        assert_eq!(prompt.decision_from_selected_index(1).unwrap(), FailedMigrationDecision::Skip);
        assert_eq!(prompt.decision_from_selected_index(2).unwrap(), FailedMigrationDecision::Abort);
    }

    #[test]
    fn stuck_prompt_maps_retry_mark_failed_abort() {
        let prompt = StuckApplyingPrompt::new(&sample_failed_record());
        assert_eq!(prompt.decision_from_selected_index(0).unwrap(), StuckApplyingDecision::Retry);
        assert_eq!(
            prompt.decision_from_selected_index(1).unwrap(),
            StuckApplyingDecision::MarkFailed
        );
        assert_eq!(prompt.decision_from_selected_index(2).unwrap(), StuckApplyingDecision::Abort);
    }
}
