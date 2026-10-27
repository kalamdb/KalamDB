//! Interactive history menu with dropdown selection
//!
//! Provides a modern CLI history browsing experience with:
//! - Dropdown menu showing multiple history items at once
//! - Arrow key navigation (Up/Down)
//! - Enter to select, Escape to cancel
//! - Search/filter functionality (type to filter)
//! - Visual highlighting of selected item

use std::io::{self, Write};

use colored::Colorize;
use crossterm::{
    cursor::{self, MoveToColumn, MoveUp},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{self, ClearType},
    ExecutableCommand,
};

/// Number of items to show in the history menu at once
const VISIBLE_ITEMS: usize = 8;

/// Result of the history menu interaction
pub enum HistoryMenuResult {
    /// User selected a command from history
    Selected(String),
    /// User cancelled (Escape or Ctrl+C)
    Cancelled,
    /// User wants to continue editing (no history item selected)
    Continue,
}

/// Interactive history menu
pub struct HistoryMenu {
    /// All history entries (newest first)
    entries: Vec<String>,
    /// Filtered entries based on search query
    filtered: Vec<usize>,
    /// Current selection index within filtered entries
    selected: usize,
    /// Scroll offset for viewing
    scroll_offset: usize,
    /// Current search/filter query
    query: String,
    /// Whether colors are enabled
    color_enabled: bool,
}

impl HistoryMenu {
    /// Create a new history menu with the given entries
    ///
    /// Entries should be provided in chronological order (oldest first).
    /// Menu displays oldest at top, newest at bottom.
    /// Selection starts at bottom (newest item).
    pub fn new(entries: Vec<String>, color_enabled: bool) -> Self {
        let mut entries_vec = entries;

        // Deduplicate consecutive entries
        entries_vec.dedup();

        let filtered: Vec<usize> = (0..entries_vec.len()).collect();

        // Start selection at the last item (newest, at bottom)
        let initial_selected = if entries_vec.is_empty() {
            0
        } else {
            entries_vec.len() - 1
        };

        Self {
            entries: entries_vec,
            filtered,
            selected: initial_selected,
            scroll_offset: 0,
            query: String::new(),
            color_enabled,
        }
    }

    /// Run the interactive history menu
    ///
    /// Returns the selected command or None if cancelled
    pub fn run(&mut self, current_input: &str) -> io::Result<HistoryMenuResult> {
        if self.entries.is_empty() {
            return Ok(HistoryMenuResult::Continue);
        }

        // Initialize query with current input for filtering
        self.query = current_input.to_string();
        self.update_filter();

        // Hide cursor before showing menu to prevent selection issues
        let mut stdout = io::stdout();
        stdout.execute(cursor::Hide)?;

        // Enable raw mode for direct keyboard input
        terminal::enable_raw_mode()?;

        let result = self.run_loop();

        // Restore terminal state
        terminal::disable_raw_mode()?;

        // Show cursor again
        stdout.execute(cursor::Show)?;

        // Clear the menu from display
        self.clear_display()?;

        result
    }

    fn run_loop(&mut self) -> io::Result<HistoryMenuResult> {
        // Initial render
        self.render()?;

        loop {
            if event::poll(std::time::Duration::from_millis(100))? {
                if let Event::Key(key_event) = event::read()? {
                    // Only handle key press events, not release
                    if key_event.kind != KeyEventKind::Press {
                        continue;
                    }

                    match key_event.code {
                        KeyCode::Esc => {
                            return Ok(HistoryMenuResult::Cancelled);
                        },
                        KeyCode::Enter => {
                            if let Some(&idx) = self.filtered.get(self.selected) {
                                if let Some(entry) = self.entries.get(idx) {
                                    return Ok(HistoryMenuResult::Selected(entry.clone()));
                                }
                            }
                            return Ok(HistoryMenuResult::Cancelled);
                        },
                        KeyCode::Up => {
                            self.move_up();
                            self.render()?;
                        },
                        KeyCode::Down => {
                            self.move_down();
                            self.render()?;
                        },
                        KeyCode::PageUp => {
                            for _ in 0..VISIBLE_ITEMS {
                                self.move_up();
                            }
                            self.render()?;
                        },
                        KeyCode::PageDown => {
                            for _ in 0..VISIBLE_ITEMS {
                                self.move_down();
                            }
                            self.render()?;
                        },
                        KeyCode::Home => {
                            self.selected = 0;
                            self.scroll_offset = 0;
                            self.render()?;
                        },
                        KeyCode::End => {
                            if !self.filtered.is_empty() {
                                self.selected = self.filtered.len() - 1;
                                self.adjust_scroll();
                            }
                            self.render()?;
                        },
                        KeyCode::Char('c')
                            if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            return Ok(HistoryMenuResult::Cancelled);
                        },
                        KeyCode::Char('n')
                            if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            self.move_down();
                            self.render()?;
                        },
                        KeyCode::Char('p')
                            if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            self.move_up();
                            self.render()?;
                        },
                        KeyCode::Char(c) => {
                            self.query.push(c);
                            self.update_filter();
                            self.render()?;
                        },
                        KeyCode::Backspace => {
                            self.query.pop();
                            self.update_filter();
                            self.render()?;
                        },
                        _ => {},
                    }
                }
            }
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.adjust_scroll();
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
            self.adjust_scroll();
        }
    }

    fn adjust_scroll(&mut self) {
        // Adjust scroll to keep selection visible
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + VISIBLE_ITEMS {
            self.scroll_offset = self.selected - VISIBLE_ITEMS + 1;
        }
    }

    fn update_filter(&mut self) {
        let query_lower = self.query.to_lowercase();

        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query_lower.is_empty() || entry.to_lowercase().contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect();

        // Reset selection to the last item (newest, at bottom) if out of bounds
        if self.filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }

        // Adjust scroll to show selection at bottom
        self.adjust_scroll();
    }

    /// Format an entry for display in the menu
    /// For multi-line commands, show first 2 lines on separate lines
    fn format_entry_for_display(entry: &str, max_width: usize) -> String {
        let lines: Vec<&str> = entry.lines().collect();

        if lines.len() <= 1 {
            // Single line - just truncate if needed
            let line = lines.first().map(|s| *s).unwrap_or("");
            if line.len() > max_width {
                format!("{}...", &line[..max_width.saturating_sub(3)])
            } else {
                line.to_string()
            }
        } else {
            // Multi-line - show first 2 lines
            let line1 = lines[0];
            let line2 = lines.get(1).map(|s| *s).unwrap_or("");

            // Truncate each line if needed
            let truncate = |s: &str| -> String {
                let trimmed = s.trim();
                if trimmed.len() > max_width.saturating_sub(4) {
                    format!("{}...", &trimmed[..max_width.saturating_sub(7)])
                } else {
                    trimmed.to_string()
                }
            };

            let display1 = truncate(line1);
            let display2 = truncate(line2);

            // Show continuation indicator if there are more lines
            let more = if lines.len() > 2 { " ⋯" } else { "" };

            format!("{}\n      {}{}", display1, display2, more)
        }
    }

    /// Render the menu to stdout
    ///
    /// Displays:
    /// - Header with count and instructions
    /// - Visible entries (up to VISIBLE_ITEMS)
    /// - Optional scroll indicator at top if there are hidden items above
    fn render(&self) -> io::Result<()> {
        let mut stdout = io::stdout();

        // Move cursor to beginning
        stdout.execute(MoveToColumn(0))?;

        let visible_count = VISIBLE_ITEMS.min(self.filtered.len());

        // === HEADER LINE ===
        self.render_header(&mut stdout)?;

        // === SCROLL INDICATOR (TOP) ===
        // Show if there are hidden items above the visible window
        let has_hidden_above = self.scroll_offset > 0;
        if has_hidden_above {
            self.render_scroll_indicator(&mut stdout, self.scroll_offset)?;
        }

        // === VISIBLE ENTRIES ===
        let extra_lines = self.render_entries(&mut stdout, visible_count)?;

        stdout.flush()?;

        // === MOVE CURSOR BACK UP ===
        // Calculate total lines: header + scroll_top + entries + extra_lines
        let scroll_top_lines = if has_hidden_above { 1 } else { 0 };
        let lines_printed = 1 + scroll_top_lines + visible_count.max(1) + extra_lines;
        stdout.execute(MoveUp(lines_printed as u16))?;

        Ok(())
    }

    /// Render the header line with count and instructions
    fn render_header(&self, stdout: &mut impl Write) -> io::Result<()> {
        stdout.execute(terminal::Clear(ClearType::CurrentLine))?;

        let header = if self.color_enabled {
            format!(
                "{}{}{}",
                " History ".on_bright_blue().white().bold(),
                format!(" ({}/{}) ", self.filtered.len(), self.entries.len()).dimmed(),
                if !self.query.is_empty() {
                    format!("filter: {}", self.query.cyan())
                } else {
                    "↑↓ navigate, Enter select, Esc cancel, type to filter".dimmed().to_string()
                }
            )
        } else {
            format!(
                " History ({}/{}) {}",
                self.filtered.len(),
                self.entries.len(),
                if !self.query.is_empty() {
                    format!("filter: {}", self.query)
                } else {
                    "↑↓ navigate, Enter select, Esc cancel".to_string()
                }
            )
        };

        writeln!(stdout, "\r{}", header)?;
        Ok(())
    }

    /// Render scroll indicator showing hidden items count
    fn render_scroll_indicator(
        &self,
        stdout: &mut impl Write,
        hidden_count: usize,
    ) -> io::Result<()> {
        stdout.execute(terminal::Clear(ClearType::CurrentLine))?;

        let indicator = if self.color_enabled {
            format!("\r  {}", format!("...{} older items above", hidden_count).dimmed())
        } else {
            format!("\r  ...{} older items above", hidden_count)
        };

        writeln!(stdout, "{}", indicator)?;
        Ok(())
    }

    /// Render visible entries and return count of extra lines from multi-line entries
    fn render_entries(&self, stdout: &mut impl Write, visible_count: usize) -> io::Result<usize> {
        let mut extra_lines = 0;

        if self.filtered.is_empty() {
            stdout.execute(terminal::Clear(ClearType::CurrentLine))?;
            let no_matches = if self.color_enabled {
                "  (no matching history entries)".dimmed().to_string()
            } else {
                "  (no matching history entries)".to_string()
            };
            writeln!(stdout, "\r{}", no_matches)?;
            return Ok(0);
        }

        let max_width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80) - 6;

        for i in 0..visible_count {
            let display_idx = self.scroll_offset + i;
            if display_idx >= self.filtered.len() {
                break;
            }

            let entry_idx = self.filtered[display_idx];
            let entry = &self.entries[entry_idx];
            let is_selected = display_idx == self.selected;

            // Format the entry for display
            let display_entry = Self::format_entry_for_display(entry, max_width);
            let newline_count = display_entry.matches('\n').count();
            extra_lines += newline_count;

            // Clear space for multi-line entries
            self.clear_entry_space(stdout, newline_count)?;

            // Format and write the entry
            let formatted = self.format_entry_lines(&display_entry, is_selected);
            writeln!(stdout, "{}", formatted)?;
        }

        Ok(extra_lines)
    }

    /// Clear terminal space for a multi-line entry
    fn clear_entry_space(&self, stdout: &mut impl Write, newline_count: usize) -> io::Result<()> {
        stdout.execute(terminal::Clear(ClearType::CurrentLine))?;

        for _ in 0..newline_count {
            stdout.execute(cursor::MoveDown(1))?;
            stdout.execute(terminal::Clear(ClearType::CurrentLine))?;
        }

        for _ in 0..newline_count {
            stdout.execute(MoveUp(1))?;
        }

        Ok(())
    }

    /// Format entry lines with proper styling and indentation
    fn format_entry_lines(&self, display_entry: &str, is_selected: bool) -> String {
        if !display_entry.contains('\n') {
            // Single line entry
            return self.format_single_line(display_entry, is_selected);
        }

        // Multi-line entry - format each line
        display_entry
            .lines()
            .enumerate()
            .map(|(idx, line)| {
                if idx == 0 {
                    self.format_single_line(line, is_selected)
                } else {
                    self.format_continuation_line(line, is_selected)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Format a single line or first line of entry
    fn format_single_line(&self, line: &str, is_selected: bool) -> String {
        if self.color_enabled {
            if is_selected {
                format!("\r  {} {}", "▸".bright_cyan().bold(), line.white().bold())
            } else {
                format!("\r    {}", line.dimmed())
            }
        } else if is_selected {
            format!("\r  > {}", line)
        } else {
            format!("\r    {}", line)
        }
    }

    /// Format a continuation line (2nd+ line of multi-line entry)
    fn format_continuation_line(&self, line: &str, is_selected: bool) -> String {
        if self.color_enabled {
            if is_selected {
                format!("\r    {}", line.white().bold())
            } else {
                format!("\r    {}", line.dimmed())
            }
        } else {
            format!("\r    {}", line)
        }
    }

    /// Calculate the number of extra lines needed for multi-line entries in the current view
    fn count_extra_lines(&self) -> usize {
        let visible_count = VISIBLE_ITEMS.min(self.filtered.len());
        let max_width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80) - 6;

        let mut extra = 0;
        for i in 0..visible_count {
            let display_idx = self.scroll_offset + i;
            if display_idx >= self.filtered.len() {
                break;
            }
            let entry_idx = self.filtered[display_idx];
            let entry = &self.entries[entry_idx];
            let display_entry = Self::format_entry_for_display(entry, max_width);
            extra += display_entry.matches('\n').count();
        }
        extra
    }

    /// Clear the menu display from the terminal
    fn clear_display(&self) -> io::Result<()> {
        let mut stdout = io::stdout();

        let visible_count = VISIBLE_ITEMS.min(self.filtered.len()).max(1);
        let extra_lines = self.count_extra_lines();

        // Calculate lines: header + scroll_top + entries + extra_lines
        let has_hidden_above = self.scroll_offset > 0;
        let scroll_top_lines = if has_hidden_above { 1 } else { 0 };
        let lines_to_clear = 1 + scroll_top_lines + visible_count + extra_lines;

        // Clear each line
        for _ in 0..lines_to_clear {
            stdout.execute(terminal::Clear(ClearType::CurrentLine))?;
            stdout.execute(cursor::MoveDown(1))?;
        }

        // Move back up
        stdout.execute(MoveUp(lines_to_clear as u16))?;
        stdout.execute(MoveToColumn(0))?;
        stdout.flush()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history_menu_creation() {
        let entries = vec![
            "SELECT * FROM users".to_string(),
            "INSERT INTO logs VALUES (1)".to_string(),
            "CREATE TABLE test (id INT)".to_string(),
        ];

        let menu = HistoryMenu::new(entries.clone(), true);

        // Should NOT be reversed - oldest first, newest last
        assert_eq!(menu.entries.len(), 3);
        assert_eq!(menu.entries[0], "SELECT * FROM users"); // oldest
        assert_eq!(menu.entries[2], "CREATE TABLE test (id INT)"); // newest

        // Selection should start at the bottom (newest item, index 2)
        assert_eq!(menu.selected, 2);
    }

    #[test]
    fn test_filter_update() {
        let entries = vec![
            "SELECT * FROM users".to_string(),
            "INSERT INTO logs VALUES (1)".to_string(),
            "SELECT id FROM orders".to_string(),
        ];

        let mut menu = HistoryMenu::new(entries, true);

        // All entries match initially
        assert_eq!(menu.filtered.len(), 3);

        // Filter by "SELECT"
        menu.query = "SELECT".to_string();
        menu.update_filter();
        assert_eq!(menu.filtered.len(), 2);

        // Filter by "INSERT"
        menu.query = "INSERT".to_string();
        menu.update_filter();
        assert_eq!(menu.filtered.len(), 1);

        // No matches
        menu.query = "DELETE".to_string();
        menu.update_filter();
        assert_eq!(menu.filtered.len(), 0);
    }

    #[test]
    fn test_navigation() {
        let entries = vec![
            "cmd1".to_string(),
            "cmd2".to_string(),
            "cmd3".to_string(),
            "cmd4".to_string(),
            "cmd5".to_string(),
        ];

        let mut menu = HistoryMenu::new(entries, false);

        // Should start at bottom (index 4, newest)
        assert_eq!(menu.selected, 4);

        // Moving up goes to older items (toward 0)
        menu.move_up();
        assert_eq!(menu.selected, 3);

        menu.move_up();
        menu.move_up();
        assert_eq!(menu.selected, 1);

        // Moving down goes to newer items (toward end)
        menu.move_down();
        assert_eq!(menu.selected, 2);

        // Should not go below 0
        menu.selected = 0;
        menu.move_up();
        assert_eq!(menu.selected, 0);

        // Should not exceed length
        menu.selected = 4;
        menu.move_down();
        assert_eq!(menu.selected, 4);
    }

    #[test]
    fn test_deduplication() {
        let entries = vec![
            "SELECT 1".to_string(),
            "SELECT 1".to_string(),
            "SELECT 2".to_string(),
            "SELECT 2".to_string(),
            "SELECT 2".to_string(),
            "SELECT 3".to_string(),
        ];

        let menu = HistoryMenu::new(entries, true);

        // Should deduplicate consecutive entries
        assert_eq!(menu.entries.len(), 3);
    }

    #[test]
    fn test_multiline_formatting() {
        // Single line - should stay as is
        let single = HistoryMenu::format_entry_for_display("SELECT * FROM users", 80);
        assert_eq!(single, "SELECT * FROM users");

        // Two lines - should show both on separate lines
        let two_lines = HistoryMenu::format_entry_for_display("SELECT *\nFROM users", 80);
        assert!(two_lines.contains('\n'));
        assert!(two_lines.contains("SELECT *"));
        assert!(two_lines.contains("FROM users"));

        // Three+ lines - should show first two and indicator
        let three_lines =
            HistoryMenu::format_entry_for_display("SELECT *\nFROM users\nWHERE id = 1", 80);
        assert!(three_lines.contains('\n'));
        assert!(three_lines.contains("SELECT *"));
        assert!(three_lines.contains("FROM users"));
        assert!(three_lines.contains("⋯")); // continuation indicator

        // Long line - should truncate
        let long_line = "a".repeat(100);
        let truncated = HistoryMenu::format_entry_for_display(&long_line, 50);
        assert!(truncated.len() < 60);
        assert!(truncated.ends_with("..."));
    }
}
