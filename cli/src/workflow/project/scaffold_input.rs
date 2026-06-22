//! Input validation for `kalam init` scaffold generation.

use crate::error::{CLIError, Result};

const MAX_PROJECT_DISPLAY_NAME_LEN: usize = 64;

/// Validate a human-readable project display name before writing config files.
pub fn validate_project_display_name(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CLIError::ConfigurationError(
            "project name must not be empty".into(),
        ));
    }

    if trimmed.len() > MAX_PROJECT_DISPLAY_NAME_LEN {
        return Err(CLIError::ConfigurationError(format!(
            "project name must be at most {MAX_PROJECT_DISPLAY_NAME_LEN} characters"
        )));
    }

    if trimmed.contains("{{") || trimmed.contains("}}") {
        return Err(CLIError::ConfigurationError(
            "project name must not contain template markers".into(),
        ));
    }

    if trimmed.chars().any(char::is_control) {
        return Err(CLIError::ConfigurationError(
            "project name must not contain control characters".into(),
        ));
    }

    for forbidden in ['"', '\'', '\\', '\n', '\r', '\t'] {
        if trimmed.contains(forbidden) {
            return Err(CLIError::ConfigurationError(format!(
                "project name must not contain '{forbidden}'"
            )));
        }
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_project_display_name_accepts_simple_names() {
        assert_eq!(
            validate_project_display_name("my-app").unwrap(),
            "my-app"
        );
    }

    #[test]
    fn validate_project_display_name_rejects_quotes() {
        assert!(validate_project_display_name("my\"app").is_err());
    }

    #[test]
    fn validate_project_display_name_rejects_template_markers() {
        assert!(validate_project_display_name("{{evil}}").is_err());
    }
}
