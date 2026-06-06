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
};

#[derive(Debug, Clone)]
pub struct WorkflowOutput {
    pub use_color: bool,
    pub logging: WorkflowLoggingPolicy,
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
        Self { use_color, logging }
    }

    pub fn status(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.use_color {
            eprintln!("{}", line.green());
        } else {
            eprintln!("{line}");
        }
        self.append_log(line);
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.use_color {
            eprintln!("{}", line.yellow());
        } else {
            eprintln!("{line}");
        }
        self.append_log(&format!("WARN: {line}"));
    }

    pub fn error(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.use_color {
            eprintln!("{}", line.red().bold());
        } else {
            eprintln!("{line}");
        }
        self.append_log(&format!("ERROR: {line}"));
    }

    pub fn detail(&self, message: impl AsRef<str>) {
        let line = message.as_ref();
        if self.use_color {
            eprintln!("{}", line.bright_black());
        } else {
            eprintln!("{line}");
        }
        self.append_log(line);
    }

    /// Emit a prefixed line from a managed dev service.
    pub fn service_log(
        &self,
        source: &crate::workflow::dev::logs::ServiceLogSource,
        message: impl AsRef<str>,
    ) {
        let message = message.as_ref();
        let formatted = source.format_line(message, self.use_color);
        eprintln!("{formatted}");
        if self.logging.capture_process_output {
            let persisted = format!("[{}] {}", source.name, message);
            self.append_log(&redact_secrets(&persisted));
        }
    }

    fn append_log(&self, line: &str) {
        if !self.logging.file_enabled {
            return;
        }
        if let Err(e) = append_workflow_log(&self.logging.path, &redact_secrets(line)) {
            eprintln!("warning: failed to write workflow log: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_secrets_masks_sensitive_lines() {
        assert_eq!(redact_secrets("Authorization: Bearer abc"), "[REDACTED]");
        assert_eq!(redact_secrets("status ok"), "status ok");
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
