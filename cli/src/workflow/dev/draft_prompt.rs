//! Modal draft migration confirmation prompt for `kalam dev`.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    error::Result,
    output::WorkflowOutput,
    terminal_ui::SelectOption,
    workflow::{
        display_project_path,
        migration::markers,
        prompts::{read_single_key_decision, run_workflow_modal_prompt, WorkflowModalPrompt},
        WorkflowContext,
    },
};

pub const DRAFT_PROMPT_MESSAGE: &str = "Apply schema draft?";
const DRAFT_SUMMARY_LINES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftPromptDecision {
    Apply,
    Reset,
    Cancel,
}

/// Absolute path to the draft migration file for this project.
pub fn draft_migration_path(ctx: &WorkflowContext) -> PathBuf {
    ctx.config
        .migrations_dir(&ctx.project_root)
        .join(crate::workflow::migration::DRAFT_MIGRATION_FILE)
}

pub fn draft_migration_exists(path: &Path) -> bool {
    path.is_file()
}

/// Show a modal confirmation prompt for the current draft migration.
pub fn prompt_for_draft_application(
    output: &WorkflowOutput,
    project_root: &Path,
    draft_path: &Path,
) -> Result<DraftPromptDecision> {
    let summary = draft_summary_lines(draft_path);
    let draft_display = display_project_path(project_root, draft_path);
    let mut context_lines = summary;
    context_lines.push(format!("Full changes: {draft_display}"));
    run_workflow_modal_prompt(output, &DraftMigrationPrompt { context_lines })
}

struct DraftMigrationPrompt {
    context_lines: Vec<String>,
}

impl WorkflowModalPrompt for DraftMigrationPrompt {
    type Decision = DraftPromptDecision;

    fn message(&self) -> &str {
        DRAFT_PROMPT_MESSAGE
    }

    fn banner_title(&self) -> &str {
        "Schema draft pending"
    }

    fn banner_subtitle(&self) -> Option<&str> {
        Some("Review before applying.")
    }

    fn context_lines(&self) -> &[String] {
        &self.context_lines
    }

    fn select_options(&self) -> Vec<SelectOption<'static>> {
        vec![
            SelectOption::described("Apply changes", "Seal and apply this schema draft now"),
            SelectOption::described(
                "Reset and rebuild",
                "Drop the namespace, clear migrations, and apply schema.sql from scratch",
            ),
            SelectOption::described("Cancel", "Stop kalam dev without applying this draft"),
        ]
    }

    fn default_index(&self) -> usize {
        0
    }

    fn decision_from_selected_index(&self, index: usize) -> Result<Self::Decision> {
        Ok(match index {
            0 => DraftPromptDecision::Apply,
            1 => DraftPromptDecision::Reset,
            _ => DraftPromptDecision::Cancel,
        })
    }

    fn read_noninteractive_decision(&self) -> Result<Self::Decision> {
        read_single_key_decision("schema prompt input", |key| match key {
            'y' | 'Y' => Ok(DraftPromptDecision::Apply),
            'r' | 'R' => Ok(DraftPromptDecision::Reset),
            _ => Ok(DraftPromptDecision::Cancel),
        })
    }

    fn read_agent_decision(&self) -> Result<Self::Decision> {
        Ok(DraftPromptDecision::Apply)
    }
}

fn draft_summary_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .ok()
        .map(|sql| summarize_draft_sql(&sql, DRAFT_SUMMARY_LINES))
        .unwrap_or_default()
}

pub fn summarize_draft_sql(sql: &str, max_lines: usize) -> Vec<String> {
    let up = markers::extract_up_section_borrowed(sql);
    let mut lines = up
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .take(max_lines + 1)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if lines.len() > max_lines
        && lines
            .first()
            .is_some_and(|line| line.to_ascii_uppercase().starts_with("CREATE TABLE "))
    {
        let all = up
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("--"))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        return summarize_create_table(&all, max_lines);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        lines.push("...".to_string());
    }
    lines
}

fn summarize_create_table(lines: &[String], max_lines: usize) -> Vec<String> {
    if lines.len() <= max_lines {
        return lines.to_vec();
    }
    let mut summary = Vec::with_capacity(max_lines + 1);
    if let Some(first) = lines.first() {
        summary.push(first.clone());
    }
    summary.push("...".to_string());
    let tail_count = max_lines.saturating_sub(1);
    let tail: Vec<_> = lines
        .iter()
        .skip(1)
        .filter(|line| {
            let t = line.trim_end_matches(';').trim();
            t != ")" && t != ");"
        })
        .collect();
    let start = tail.len().saturating_sub(tail_count);
    for line in tail.into_iter().skip(start) {
        summary.push(line.clone());
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_limits_to_max_lines() {
        let sql = "-- UP\nALTER TABLE users ADD COLUMN a TEXT;\nALTER TABLE users ADD COLUMN b \
                   TEXT;\nALTER TABLE users ADD COLUMN c TEXT;\nALTER TABLE users ADD COLUMN d \
                   TEXT;\n-- DOWN\n";
        let result = summarize_draft_sql(sql, 3);
        assert_eq!(result.len(), 4);
        assert_eq!(result[3], "...");
    }

    #[test]
    fn summarize_create_table_shows_first_and_last_columns() {
        let sql = "-- UP\nCREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  email TEXT NOT \
                   NULL,\n  first_name TEXT,\n  last_name TEXT,\n  created_at TIMESTAMP\n);\n-- \
                   DOWN\n";
        let result = summarize_draft_sql(sql, 3);
        assert_eq!(result[0], "CREATE TABLE users (");
        assert!(result.iter().any(|l| l.contains("...")));
    }

    #[test]
    fn draft_prompt_maps_apply_reset_cancel() {
        let prompt = DraftMigrationPrompt {
            context_lines: vec!["CREATE TABLE users (id INTEGER);".into()],
        };
        assert_eq!(prompt.decision_from_selected_index(0).unwrap(), DraftPromptDecision::Apply);
        assert_eq!(prompt.decision_from_selected_index(1).unwrap(), DraftPromptDecision::Reset);
        assert_eq!(prompt.decision_from_selected_index(2).unwrap(), DraftPromptDecision::Cancel);
    }
}
