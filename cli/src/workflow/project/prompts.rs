//! Interactive prompts for workflow commands.

use std::io::{self, IsTerminal};

use crate::{
    error::{CLIError, Result},
    terminal_input,
    terminal_ui::{self, SelectOption},
};

pub fn interactive_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub fn print_workflow_banner(title: &str, subtitle: &str, color: bool) {
    terminal_ui::print_banner(title, Some(subtitle), color);
    println!();
}

pub fn prompt_text(label: &str, color: bool) -> Result<String> {
    terminal_input::prompt_line(&terminal_ui::prompt_label(label, color))
        .map_err(|e| CLIError::FileError(format!("failed to read input: {e}")))
}

pub fn prompt_text_with_default(label: &str, default: &str, color: bool) -> Result<String> {
    let line =
        terminal_input::prompt_line(&terminal_ui::prompt_label_with_default(label, default, color))
            .map_err(|e| CLIError::FileError(format!("failed to read input: {e}")))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

pub fn prompt_select(
    title: &str,
    options: &[SelectOption<'_>],
    default_index: usize,
    color: bool,
) -> Result<usize> {
    terminal_ui::prompt_select(title, options, default_index, color).map_err(prompt_error)
}

pub fn prompt_multi_select(
    title: &str,
    options: &[SelectOption<'_>],
    default_selected: &[bool],
    color: bool,
) -> Result<Vec<usize>> {
    terminal_ui::prompt_multi_select(title, options, default_selected, color).map_err(prompt_error)
}

fn prompt_error(error: io::Error) -> CLIError {
    if error.kind() == io::ErrorKind::Interrupted {
        CLIError::ConfigurationError("setup cancelled".into())
    } else {
        CLIError::FileError(format!("terminal prompt failed: {error}"))
    }
}
