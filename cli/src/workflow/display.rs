//! Shared progress vs plain-status output branching for dev workflows.

use crate::{
    output::{WorkflowDisplayMode, WorkflowOutput},
    terminal_ui::ProgressTaskStatus,
};

pub(crate) fn with_display_mode(
    output: &WorkflowOutput,
    progress: impl FnOnce(),
    plain: impl FnOnce(),
) {
    if output.display_mode == WorkflowDisplayMode::Progress {
        progress();
    } else {
        plain();
    }
}

pub(crate) fn emit_task_success(
    output: &WorkflowOutput,
    task_id: &str,
    progress_label: &str,
    status_label: &str,
) {
    with_display_mode(
        output,
        || output.progress_task(task_id, ProgressTaskStatus::Succeeded, progress_label),
        || output.status(status_label),
    );
}

pub(crate) fn emit_task_failure(
    output: &WorkflowOutput,
    task_id: &str,
    progress_label: impl std::fmt::Display,
    on_plain: impl FnOnce(),
) {
    with_display_mode(
        output,
        || output.progress_task(task_id, ProgressTaskStatus::Failed, progress_label.to_string()),
        on_plain,
    );
}
