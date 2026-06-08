//! Shared CLI output helpers for workflow commands.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use colored::Colorize;

use crate::{
    config::WorkflowLoggingPolicy,
    error::{CLIError, Result},
    terminal_ui::{ProgressTaskStatus, ProgressTasks},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowDisplayMode {
    Normal,
    Progress,
    Verbose,
}

#[derive(Debug, Clone)]
pub struct WorkflowOutput {
    pub use_color: bool,
    pub logging: WorkflowLoggingPolicy,
    pub display_mode: WorkflowDisplayMode,
    progress_tasks: Option<ProgressTasks>,
}

/// Patterns that trigger redaction before persisting workflow logs.
const SECRET_PATTERNS: &[&str] = &[
    "jwt",
    "password",
    "token",
    "authorization",
    "bearer",
    "secret",
];

impl WorkflowOutput {
    pub fn new(use_color: bool, logging: WorkflowLoggingPolicy) -> Self {
        Self {
            use_color,
            logging,
            display_mode: WorkflowDisplayMode::Normal,
            progress_tasks: None,
        }
    }

    pub fn with_display_mode(mut self, display_mode: WorkflowDisplayMode) -> Self {
        self.display_mode = display_mode;
        if display_mode == WorkflowDisplayMode::Progress && self.progress_tasks.is_none() {
            self.progress_tasks = Some(ProgressTasks::new(self.use_color));
        }
        self
    }

    pub fn status(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.display_mode == WorkflowDisplayMode::Progress {
            self.append_log(line);
        } else if self.use_color {
            eprintln!("{}", line.green());
            self.append_log(line);
        } else {
            eprintln!("{line}");
            self.append_log(line);
        }
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.display_mode == WorkflowDisplayMode::Progress {
            self.append_log(&format!("WARN: {line}"));
        } else if self.use_color {
            eprintln!("{}", line.yellow());
            self.append_log(&format!("WARN: {line}"));
        } else {
            eprintln!("{line}");
            self.append_log(&format!("WARN: {line}"));
        }
    }

    pub fn error(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.display_mode == WorkflowDisplayMode::Progress {
            self.append_log(&format!("ERROR: {line}"));
        } else if self.use_color {
            eprintln!("{}", line.red().bold());
            self.append_log(&format!("ERROR: {line}"));
        } else {
            eprintln!("{line}");
            self.append_log(&format!("ERROR: {line}"));
        }
    }

    pub fn detail(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.display_mode == WorkflowDisplayMode::Progress {
            self.append_log(line);
        } else if self.use_color {
            eprintln!("{}", line.bright_black());
            self.append_log(line);
        } else {
            eprintln!("{line}");
            self.append_log(line);
        }
    }

    /// Emit a prefixed line from a managed dev service.
    pub fn service_log(
        &self,
        source: &crate::workflow::dev::logs::ServiceLogSource,
        message: impl AsRef<str>,
    ) {
        self.emit_service_log(source, message.as_ref(), false);
    }

    pub fn process_log(
        &self,
        source: &crate::workflow::dev::logs::ServiceLogSource,
        message: impl AsRef<str>,
    ) {
        self.emit_service_log(source, message.as_ref(), true);
    }

    fn emit_service_log(
        &self,
        source: &crate::workflow::dev::logs::ServiceLogSource,
        message: &str,
        process_output: bool,
    ) {
        if self.display_mode != WorkflowDisplayMode::Progress {
            let formatted = source.format_line(message, self.use_color);
            eprintln!("{formatted}");
        } else if process_output && source.name == "server" {
            if let Some(progress_tasks) = &self.progress_tasks {
                progress_tasks.push_task_detail("server", message, 8);
            }
        }
        if self.logging.capture_process_output {
            let persisted = format!("[{}] {}", source.name, message);
            self.append_log(&redact_secrets(&persisted));
        }
    }

    pub fn progress_task(&self, id: &str, status: ProgressTaskStatus, message: impl AsRef<str>) {
        if let Some(progress_tasks) = &self.progress_tasks {
            progress_tasks.update_task(id, status, message.as_ref());
        }
        if status == ProgressTaskStatus::Failed {
            self.append_log(&format!("ERROR: {}", message.as_ref()));
        } else {
            self.append_log(message.as_ref());
        }
    }

    pub fn workflow_log_path(&self) -> Option<&Path> {
        self.logging.file_enabled.then_some(self.logging.path.as_path())
    }

    fn append_log(&self, line: &str) {
        if !self.logging.file_enabled {
            return;
        }
        if let Err(e) = append_workflow_log(&self.logging.path, &redact_secrets(line)) {
            eprintln!("warning: failed to write workflow log: {e}");
        }
    }

    #[cfg(test)]
    fn progress_lines(&self) -> Vec<String> {
        self.progress_tasks
            .as_ref()
            .map(ProgressTasks::rendered_lines)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::dev::logs::{ServiceColor, ServiceLogSource};

    #[test]
    fn redact_secrets_masks_sensitive_lines() {
        assert_eq!(redact_secrets("Authorization: Bearer abc"), "[REDACTED]");
        assert_eq!(redact_secrets("status ok"), "status ok");
    }

    #[test]
    fn server_process_log_updates_progress_task_details() {
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled())
            .with_display_mode(WorkflowDisplayMode::Progress);
        output.progress_task("server", ProgressTaskStatus::Running, "Starting server");
        let source = ServiceLogSource::new("server", ServiceColor::Cyan);

        for idx in 0..10 {
            output.process_log(&source, format!("server line {idx}"));
        }

        let lines = output.progress_lines();
        assert_eq!(lines.len(), 9);
        assert_eq!(lines[0], "⠋ Starting server");
        assert_eq!(lines[1], "    server line 2");
        assert_eq!(lines[8], "    server line 9");
    }
}

/// Redact likely secret values before writing logs to disk.
pub fn redact_secrets(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if SECRET_PATTERNS.iter().any(|pattern| lower.contains(pattern)) {
        return "[REDACTED]".to_string();
    }
    line.to_string()
}

pub fn append_workflow_log(path: &Path, line: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path).map_err(|e| {
        CLIError::FileError(format!("failed to open log '{}': {e}", path.display()))
    })?;
    writeln!(file, "{line}").map_err(|e| CLIError::FileError(e.to_string()))?;
    Ok(())
}
