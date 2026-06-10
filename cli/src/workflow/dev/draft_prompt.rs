//! Modal draft migration confirmation prompt for `kalam dev`.
//!
//! The dev command is append-only while it is running. When user input is
//! required, terminal output is paused by the caller, the screen is cleared, and
//! this module renders an init-style selector.

use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::{display_project_path, WorkflowContext},
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
    let _pause = output.pause_terminal_output();
    output.status(DRAFT_PROMPT_MESSAGE);
    clear_terminal()?;
    let summary = draft_summary_lines(draft_path);
    for line in &summary {
        output.detail(line);
    }
    let draft_display = display_project_path(project_root, draft_path);
    output.detail(format!("Full changes: {draft_display}"));
    render_draft_summary(&summary, &draft_display, output.use_color);

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return read_noninteractive_decision();
    }

    let options = [
        SelectOption::described("Apply changes", "Seal and apply this schema draft now"),
        SelectOption::described(
            "Reset and rebuild",
            "Drop the namespace, clear migrations, and apply schema.sql from scratch",
        ),
        SelectOption::described("Cancel", "Stop kalam dev without applying this draft"),
    ];
    let selected =
        match terminal_ui::prompt_select(DRAFT_PROMPT_MESSAGE, &options, 0, output.use_color) {
            Ok(selected) => selected,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => 2,
            Err(error) => return Err(prompt_error(error)),
        };

    clear_terminal()?;
    Ok(match selected {
        0 => DraftPromptDecision::Apply,
        1 => DraftPromptDecision::Reset,
        _ => DraftPromptDecision::Cancel,
    })
}

fn read_noninteractive_decision() -> Result<DraftPromptDecision> {
    let mut buf = [0_u8; 1];
    io::stdin()
        .read_exact(&mut buf)
        .map_err(|e| CLIError::FileError(format!("failed to read schema prompt input: {e}")))?;
    match buf[0] as char {
        'y' | 'Y' => Ok(DraftPromptDecision::Apply),
        'r' | 'R' => Ok(DraftPromptDecision::Reset),
        _ => Ok(DraftPromptDecision::Cancel),
    }
}

fn clear_terminal() -> Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[2J\x1b[H")
        .and_then(|_| stdout.flush())
        .map_err(|e| CLIError::FileError(format!("failed to clear terminal: {e}")))
}

fn render_draft_summary(summary: &[String], draft_display: &str, color: bool) {
    terminal_ui::print_banner("Schema draft pending", Some("Review before applying."), color);
    println!();
    println!("{}", terminal_ui::style_info("Pending changes:", color));

    if summary.is_empty() {
        println!("  _draft.sql has pending changes");
    } else {
        for line in summary {
            println!("  {line}");
        }
    }
    println!();
    println!("Full changes: {draft_display}");
    println!();
}

fn draft_summary_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .ok()
        .map(|sql| summarize_draft_sql(&sql, DRAFT_SUMMARY_LINES))
        .unwrap_or_default()
}

fn prompt_error(error: io::Error) -> CLIError {
    CLIError::FileError(format!("terminal prompt failed: {error}"))
}

pub fn summarize_draft_sql(sql: &str, max_lines: usize) -> Vec<String> {
    let up = extract_up_section(sql);
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

fn extract_up_section(sql: &str) -> &str {
    let Some(start) = find_marker_end(sql, "-- UP") else {
        return sql.trim();
    };
    let rest = &sql[start..];
    let end = find_marker_start(rest, "-- DOWN").unwrap_or(rest.len());
    rest[..end].trim()
}

fn find_marker_end(sql: &str, marker: &str) -> Option<usize> {
    let mut offset = 0usize;
    for segment in sql.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        if line.trim().eq_ignore_ascii_case(marker) {
            return Some(offset + segment.len());
        }
        offset += segment.len();
    }
    if sql[offset..].trim().eq_ignore_ascii_case(marker) {
        return Some(sql.len());
    }
    None
}

fn find_marker_start(sql: &str, marker: &str) -> Option<usize> {
    let mut offset = 0usize;
    for segment in sql.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        if line.trim().eq_ignore_ascii_case(marker) {
            return Some(offset);
        }
        offset += segment.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_limits_to_max_lines() {
        let sql = "-- UP\nALTER TABLE users ADD COLUMN a TEXT;\nALTER TABLE users ADD COLUMN b TEXT;\nALTER TABLE users ADD COLUMN c TEXT;\nALTER TABLE users ADD COLUMN d TEXT;\n-- DOWN\n";
        let result = summarize_draft_sql(sql, 3);
        assert_eq!(result.len(), 4);
        assert_eq!(result[3], "...");
    }

    #[test]
    fn summarize_create_table_shows_first_and_last_columns() {
        let sql = "-- UP\nCREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  email TEXT NOT NULL,\n  first_name TEXT,\n  last_name TEXT,\n  created_at TIMESTAMP\n);\n-- DOWN\n";
        let result = summarize_draft_sql(sql, 3);
        assert_eq!(result[0], "CREATE TABLE users (");
        assert!(result.iter().any(|l| l.contains("...")));
    }
}
