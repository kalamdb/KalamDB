//! Shared CLI output helpers for workflow commands.

use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use colored::Colorize;

use crate::{
    config::WorkflowLoggingPolicy,
    error::{CLIError, Result},
    terminal_ui::{self, ProgressTaskStatus, ProgressTasks},
};

const TERMINAL_LOG_BUFFER_CAPACITY: usize = 100;
const TERMINAL_CLI_VIEW_LINES: usize = 5;
const TERMINAL_SERVICE_VIEW_LINES: usize = 5;

#[derive(Debug)]
struct TerminalLogBuffer {
    cli_lines: VecDeque<String>,
    service_lines: VecDeque<String>,
}

impl TerminalLogBuffer {
    fn new() -> Self {
        Self {
            cli_lines: VecDeque::with_capacity(TERMINAL_LOG_BUFFER_CAPACITY),
            service_lines: VecDeque::with_capacity(TERMINAL_LOG_BUFFER_CAPACITY),
        }
    }

    fn push_cli(&mut self, line: String) {
        push_bounded(&mut self.cli_lines, line);
    }

    fn push_service(&mut self, line: String) {
        push_bounded(&mut self.service_lines, line);
    }

    fn rendered_lines(&self) -> Vec<String> {
        let mut lines =
            Vec::with_capacity(TERMINAL_CLI_VIEW_LINES + TERMINAL_SERVICE_VIEW_LINES + 1);
        lines.extend(tail_lines(&self.cli_lines, TERMINAL_CLI_VIEW_LINES));
        if !self.cli_lines.is_empty() && !self.service_lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(tail_lines(&self.service_lines, TERMINAL_SERVICE_VIEW_LINES));
        lines
    }

    #[cfg(test)]
    fn all_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.cli_lines.len() + self.service_lines.len());
        lines.extend(self.cli_lines.iter().cloned());
        lines.extend(self.service_lines.iter().cloned());
        lines
    }
}

fn push_bounded(lines: &mut VecDeque<String>, line: String) {
    if lines.len() >= TERMINAL_LOG_BUFFER_CAPACITY {
        lines.pop_front();
    }
    lines.push_back(line);
}

fn tail_lines(lines: &VecDeque<String>, max_lines: usize) -> impl Iterator<Item = String> + '_ {
    lines.iter().skip(lines.len().saturating_sub(max_lines)).cloned()
}

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
    terminal_paused: Arc<AtomicBool>,
    terminal_log_buffer: Arc<Mutex<TerminalLogBuffer>>,
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
            terminal_paused: Arc::new(AtomicBool::new(false)),
            terminal_log_buffer: Arc::new(Mutex::new(TerminalLogBuffer::new())),
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
            return;
        }
        let formatted = if self.use_color {
            line.green().to_string()
        } else {
            line.to_string()
        };
        self.emit_cli_line(&formatted, line);
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.display_mode == WorkflowDisplayMode::Progress {
            self.append_log(&format!("WARN: {line}"));
            return;
        }
        let persisted = format!("WARN: {line}");
        let formatted = if self.use_color {
            line.yellow().to_string()
        } else {
            line.to_string()
        };
        self.emit_cli_line(&formatted, &persisted);
    }

    pub fn error(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.display_mode == WorkflowDisplayMode::Progress {
            self.append_log(&format!("ERROR: {line}"));
            return;
        }
        let persisted = format!("ERROR: {line}");
        let formatted = if self.use_color {
            line.red().bold().to_string()
        } else {
            line.to_string()
        };
        self.emit_cli_line(&formatted, &persisted);
    }

    pub fn detail(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.display_mode == WorkflowDisplayMode::Progress {
            self.append_log(line);
            return;
        }
        let formatted = if self.use_color {
            line.bright_black().to_string()
        } else {
            line.to_string()
        };
        self.emit_cli_line(&formatted, line);
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
        let formatted = source.format_line(message, self.use_color);
        if self.display_mode == WorkflowDisplayMode::Progress {
            if process_output && source.name == "server" {
                if let Some(progress_tasks) = &self.progress_tasks {
                    progress_tasks.push_task_detail("server", message, 8);
                }
            }
        } else {
            self.emit_service_line(&formatted, &formatted);
        }
        if self.logging.capture_process_output {
            let persisted = format!("[{}] {}", source.name, message);
            self.append_log(&redact_secrets(&persisted));
        }
    }

    pub fn progress_task(&self, id: &str, status: ProgressTaskStatus, message: impl AsRef<str>) {
        if !self.terminal_output_paused() {
            if let Some(progress_tasks) = &self.progress_tasks {
                progress_tasks.update_task(id, status, message.as_ref());
            }
        }
        if status == ProgressTaskStatus::Failed {
            self.append_log(&format!("ERROR: {}", message.as_ref()));
        } else {
            self.append_log(message.as_ref());
        }
    }

    /// Clear all detail lines for a progress task.
    ///
    /// Use this before a state transition so detail lines from a previous phase
    /// (e.g. pipeline output) do not bleed into the next (e.g. draft prompt).
    pub fn clear_progress_details(&self, id: &str) {
        if let Some(progress_tasks) = &self.progress_tasks {
            progress_tasks.clear_task_details(id);
        }
    }

    pub fn progress_detail(&self, id: &str, message: impl AsRef<str>) {
        let line = message.as_ref();
        if !self.terminal_output_paused() {
            if let Some(progress_tasks) = &self.progress_tasks {
                progress_tasks.push_task_detail(id, line, 8);
                self.append_log(line);
                return;
            }
        }
        self.detail(line);
    }

    pub fn workflow_log_path(&self) -> Option<&Path> {
        self.logging.file_enabled.then_some(self.logging.path.as_path())
    }

    pub fn workflow_event(&self, message: impl AsRef<str>) {
        self.append_log(message.as_ref());
    }

    pub fn suspend_progress<F: FnOnce() -> R, R>(&self, f: F) -> R {
        if let Some(progress_tasks) = &self.progress_tasks {
            progress_tasks.suspend(f)
        } else {
            f()
        }
    }

    pub fn pause_terminal_output(&self) -> TerminalOutputPause {
        self.terminal_paused.store(true, Ordering::SeqCst);
        TerminalOutputPause {
            paused: Arc::clone(&self.terminal_paused),
        }
    }

    pub fn run_terminal_modal<F: FnOnce() -> Result<R>, R>(&self, f: F) -> Result<R> {
        let result = self.suspend_progress(|| {
            let _pause = self.pause_terminal_output();
            terminal_ui::clear_terminal().map_err(clear_terminal_error)?;
            let result = f();
            let clear_result = terminal_ui::clear_terminal().map_err(clear_terminal_error);
            match (result, clear_result) {
                (Ok(value), Ok(())) => Ok(value),
                (Err(error), _) => Err(error),
                (Ok(_), Err(error)) => Err(error),
            }
        });
        let restore_result = self.restore_dev_session_view();
        match (result, restore_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Replay the recent dev-session log view after a modal prompt clears the screen.
    pub fn restore_dev_session_view(&self) -> Result<()> {
        if self.display_mode == WorkflowDisplayMode::Progress {
            if let Some(progress_tasks) = &self.progress_tasks {
                if io::stdout().is_terminal() {
                    terminal_ui::clear_terminal().map_err(|error| {
                        CLIError::FileError(format!("failed to clear terminal: {error}"))
                    })?;
                }
                for line in progress_tasks.rendered_lines() {
                    eprintln!("{line}");
                }
            }
            return Ok(());
        }

        let lines = self.rendered_terminal_view_lines();
        if lines.is_empty() || !io::stderr().is_terminal() {
            return Ok(());
        }

        terminal_ui::clear_terminal()
            .map_err(|error| CLIError::FileError(format!("failed to clear terminal: {error}")))?;
        for line in lines {
            eprintln!("{line}");
        }
        Ok(())
    }

    fn emit_cli_line(&self, formatted: &str, persisted: &str) {
        self.emit_display_line(&format_cli_line(formatted), TerminalLineKind::Cli);
        self.append_log(persisted);
    }

    fn emit_service_line(&self, formatted: &str, persisted: &str) {
        self.emit_display_line(&formatted_service_line(formatted), TerminalLineKind::Service);
        self.append_log(persisted);
    }

    fn emit_display_line(&self, line: &str, kind: TerminalLineKind) {
        self.buffer_terminal_line(line, kind);
        if !self.terminal_output_paused() {
            eprintln!("{line}");
        }
    }

    fn buffer_terminal_line(&self, line: &str, kind: TerminalLineKind) {
        let mut buffer = self.terminal_log_buffer.lock().expect("terminal log buffer");
        match kind {
            TerminalLineKind::Cli => buffer.push_cli(line.to_string()),
            TerminalLineKind::Service => buffer.push_service(line.to_string()),
        }
    }

    fn rendered_terminal_view_lines(&self) -> Vec<String> {
        self.terminal_log_buffer.lock().expect("terminal log buffer").rendered_lines()
    }

    fn terminal_output_paused(&self) -> bool {
        self.terminal_paused.load(Ordering::SeqCst)
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

    #[cfg(test)]
    fn buffered_terminal_lines(&self) -> Vec<String> {
        self.terminal_log_buffer.lock().expect("terminal log buffer").all_lines()
    }
}

pub struct TerminalOutputPause {
    paused: Arc<AtomicBool>,
}

impl Drop for TerminalOutputPause {
    fn drop(&mut self) {
        self.paused.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy)]
enum TerminalLineKind {
    Cli,
    Service,
}

fn format_cli_line(line: &str) -> String {
    format!("[cli] {line}")
}

fn formatted_service_line(line: &str) -> String {
    match line.split_once(' ') {
        Some((prefix, message)) => format!("{prefix}\t{message}"),
        None => line.to_string(),
    }
}

fn clear_terminal_error(error: io::Error) -> CLIError {
    CLIError::FileError(format!("failed to clear terminal: {error}"))
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

    #[test]
    fn progress_detail_pushes_task_detail_lines() {
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled())
            .with_display_mode(WorkflowDisplayMode::Progress);
        output.progress_task("schema", ProgressTaskStatus::Running, "Applying schema");
        output.progress_detail("schema", "sealed draft migration 0001_auto.sql");

        let lines = output.progress_lines();
        assert_eq!(lines[0], "⠋ Applying schema");
        assert_eq!(lines[1], "    sealed draft migration 0001_auto.sql");
    }

    #[test]
    fn terminal_log_buffer_keeps_latest_service_lines() {
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let source = ServiceLogSource::new("server", ServiceColor::Cyan);

        for idx in 0..TERMINAL_LOG_BUFFER_CAPACITY + 5 {
            output.service_log(&source, format!("server line {idx}"));
        }

        let buffered = output.buffered_terminal_lines();
        assert_eq!(buffered.len(), TERMINAL_LOG_BUFFER_CAPACITY);
        assert_eq!(buffered[0], "[server]\tserver line 5");
        assert_eq!(buffered.last().map(String::as_str), Some("[server]\tserver line 104"));
    }

    #[test]
    fn terminal_view_preserves_service_region_when_cli_output_overflows() {
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let source = ServiceLogSource::new("server", ServiceColor::Cyan);
        output.service_log(&source, "server still running");

        for idx in 0..TERMINAL_LOG_BUFFER_CAPACITY + 20 {
            output.status(format!("migration output {idx}"));
        }

        let rendered = output.rendered_terminal_view_lines();
        assert!(rendered.iter().any(|line| line == "[server]\tserver still running"));
        assert!(rendered.iter().any(|line| line == "[cli] migration output 119"));
    }

    #[test]
    fn workflow_event_does_not_change_terminal_view() {
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let source = ServiceLogSource::new("server", ServiceColor::Cyan);
        output.service_log(&source, "server ready");

        output.workflow_event("Apply schema draft?");
        output.status("applied 1 migration(s)");

        assert_eq!(
            output.rendered_terminal_view_lines(),
            vec!["[cli] applied 1 migration(s)", "", "[server]\tserver ready"]
        );
    }

    #[test]
    fn terminal_modal_keeps_service_logs_emitted_while_prompt_is_open() {
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let source = ServiceLogSource::new("server", ServiceColor::Cyan);
        output.service_log(&source, "server ready");

        output
            .run_terminal_modal(|| {
                output.workflow_event("Apply schema draft?");
                output.service_log(&source, "server tick during prompt");
                Ok(())
            })
            .unwrap();
        output.status("applied 1 migration(s)");

        assert_eq!(
            output.rendered_terminal_view_lines(),
            vec![
                "[cli] applied 1 migration(s)",
                "",
                "[server]\tserver ready",
                "[server]\tserver tick during prompt"
            ]
        );
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
