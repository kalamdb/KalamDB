use std::{
    collections::VecDeque,
    fmt::{self, Display},
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use colored::Colorize;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{self, ClearType},
    ExecutableCommand,
};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::terminal_input;

const SPINNER_TICKS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_TICK_INTERVAL_MS: u64 = 80;
const PROGRESS_DETAIL_INDENT: &str = "    ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressTaskStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ProgressTask {
    pub id:      String,
    pub message: String,
    pub status:  ProgressTaskStatus,
    pub details: Vec<String>,
}

#[derive(Clone)]
pub struct ProgressTasks {
    inner: Arc<Mutex<ProgressTasksState>>,
}

impl fmt::Debug for ProgressTasks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgressTasks").finish_non_exhaustive()
    }
}

struct ProgressTasksState {
    tasks: Vec<ProgressTask>,
    bars:  Vec<ProgressTaskBars>,
    multi: MultiProgress,
    color: bool,
}

struct ProgressTaskBars {
    id:      String,
    bar:     ProgressBar,
    details: VecDeque<ProgressBar>,
}

#[derive(Debug, Clone, Copy)]
pub struct SelectOption<'a> {
    pub label:       &'a str,
    pub description: Option<&'a str>,
    pub disabled:    bool,
}

impl<'a> SelectOption<'a> {
    pub const fn new(label: &'a str) -> Self {
        Self {
            label,
            description: None,
            disabled: false,
        }
    }

    pub const fn described(label: &'a str, description: &'a str) -> Self {
        Self {
            label,
            description: Some(description),
            disabled: false,
        }
    }

    pub const fn disabled(label: &'a str, description: &'a str) -> Self {
        Self {
            label,
            description: Some(description),
            disabled: true,
        }
    }
}

/// Clear the entire terminal screen and move the cursor to the home row.
pub fn clear_terminal() -> io::Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[2J\x1b[H")?;
    stdout.flush()
}

pub fn print_banner(title: &str, subtitle: Option<&str>, color: bool) {
    for line in banner_lines(title, subtitle, color) {
        println!("{line}");
    }
}

pub fn banner_lines(title: &str, subtitle: Option<&str>, color: bool) -> Vec<String> {
    let normalized_title = format!(" {} ", title.trim().to_ascii_uppercase());
    let border = "=".repeat(normalized_title.len());
    let mut lines = Vec::with_capacity(if subtitle.is_some() { 4 } else { 3 });

    lines.push(style_accent(&border, color));
    lines.push(if color {
        normalized_title.white().bold().on_blue().to_string()
    } else {
        normalized_title
    });
    lines.push(style_accent(&border, color));

    if let Some(subtitle) = subtitle {
        lines.push(style_info(subtitle, color));
    }

    lines
}

pub fn prompt_label(label: &str, color: bool) -> String {
    let label = label.trim_end();
    if color {
        format!("{} ", label.bright_cyan().bold())
    } else {
        format!("{label} ")
    }
}

pub fn prompt_label_with_default(label: &str, default: &str, color: bool) -> String {
    let label = label.trim_end_matches(':').trim_end();
    let text = format!("{label} [{default}]:");
    prompt_label(&text, color)
}

/// Ring the terminal bell so the user notices input is required in another tab.
pub fn alert_action_required() {
    let _ = alert_action_required_inner();
}

fn alert_action_required_inner() -> io::Result<()> {
    let mut stderr = io::stderr();
    stderr.write_all(b"\x07")?;
    stderr.flush()
}

pub fn prompt_confirm(prompt: &str, default: bool, color: bool) -> io::Result<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    let response =
        terminal_input::prompt_line(&prompt_label(&format!("{prompt} {suffix}:"), color))?;
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }

    Ok(matches!(trimmed.to_ascii_lowercase().as_str(), "y" | "yes"))
}

pub fn style_value(value: &str, color: bool) -> String {
    if color {
        value.bright_cyan().bold().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_success(value: &str, color: bool) -> String {
    if color {
        value.green().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_warning(value: &str, color: bool) -> String {
    if color {
        value.yellow().bold().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_muted(value: &str, color: bool) -> String {
    if color {
        value.bright_black().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_info(value: &str, color: bool) -> String {
    if color {
        value.bright_white().to_string()
    } else {
        value.to_string()
    }
}

pub fn style_accent(value: &str, color: bool) -> String {
    if color {
        value.bright_blue().bold().to_string()
    } else {
        value.to_string()
    }
}

impl ProgressTasks {
    pub fn new(color: bool) -> Self {
        Self::with_draw_target(color, ProgressDrawTarget::stderr())
    }

    fn with_draw_target(color: bool, draw_target: ProgressDrawTarget) -> Self {
        let multi = MultiProgress::with_draw_target(draw_target);
        Self {
            inner: Arc::new(Mutex::new(ProgressTasksState {
                tasks: Vec::new(),
                bars: Vec::new(),
                multi,
                color,
            })),
        }
    }

    pub fn update_task(
        &self,
        id: impl Into<String>,
        status: ProgressTaskStatus,
        message: impl Into<String>,
    ) {
        let mut state = self.inner.lock().expect("progress task state");
        let id = id.into();
        let message = message.into();
        if let Some(task) = state.tasks.iter_mut().find(|task| task.id == id) {
            task.status = status;
            task.message = message;
        } else {
            state.tasks.push(ProgressTask {
                id: id.clone(),
                message,
                status,
                details: Vec::new(),
            });
        }
        sync_progress_task_bar(&mut state, &id);
    }

    pub fn push_task_detail(&self, id: &str, detail: impl Into<String>, max_details: usize) {
        let mut state = self.inner.lock().expect("progress task state");
        let Some(task) = state.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        let detail = detail.into();
        task.details.push(detail.clone());
        if task.details.len() > max_details {
            let overflow = task.details.len() - max_details;
            task.details.drain(0..overflow);
        }
        sync_progress_task_detail_bars(&mut state, id, max_details);
    }

    pub fn rendered_lines(&self) -> Vec<String> {
        let state = self.inner.lock().expect("progress task state");
        render_progress_tasks(&state.tasks, state.color)
    }

    /// Remove all detail bars for a task and clear the task's detail list.
    /// Call this before a state transition so stale detail lines from a
    /// previous phase do not carry over into the next one.
    pub fn clear_task_details(&self, id: &str) {
        let mut state = self.inner.lock().expect("progress task state");
        if let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) {
            task.details.clear();
        }
        if let Some(bars_index) = state.bars.iter().position(|b| b.id == id) {
            while let Some(bar) = state.bars[bars_index].details.pop_front() {
                // Remove from MultiProgress entirely so it does not leave a
                // "ghost" empty slot that would push subsequent detail lines
                // (including the "Apply? [y/N]:" prompt) off-screen on the
                // second and subsequent draft-prompt cycles.
                state.multi.remove(&bar);
            }
        }
    }

    pub fn suspend<F: FnOnce() -> R, R>(&self, f: F) -> R {
        let state = self.inner.lock().expect("progress task state");
        state.multi.suspend(f)
    }

    #[cfg(test)]
    fn new_hidden(color: bool) -> Self {
        Self::with_draw_target(color, ProgressDrawTarget::hidden())
    }
}

pub fn render_progress_task(task: &ProgressTask, color: bool) -> Vec<String> {
    let mut lines = Vec::with_capacity(1 + task.details.len());
    lines.push(render_progress_task_line(task, color));
    lines.extend(task.details.iter().map(|detail| render_progress_detail_line(detail, color)));
    lines
}

fn render_progress_task_line(task: &ProgressTask, color: bool) -> String {
    let symbol = match task.status {
        ProgressTaskStatus::Running => "⠋",
        ProgressTaskStatus::Succeeded => "✓",
        ProgressTaskStatus::Failed => "✗",
    };
    let line = format!("{symbol} {}", task.message);
    match task.status {
        ProgressTaskStatus::Running => line,
        ProgressTaskStatus::Succeeded => style_success(&line, color),
        ProgressTaskStatus::Failed => {
            if color {
                line.red().bold().to_string()
            } else {
                line
            }
        },
    }
}

fn render_progress_detail_line(detail: &str, color: bool) -> String {
    let line = format!("{PROGRESS_DETAIL_INDENT}{detail}");
    style_muted(&line, color)
}

fn render_progress_tasks(tasks: &[ProgressTask], color: bool) -> Vec<String> {
    tasks.iter().flat_map(|task| render_progress_task(task, color)).collect()
}

fn sync_progress_task_bar(state: &mut ProgressTasksState, id: &str) {
    let Some(task) = state.tasks.iter().find(|task| task.id == id) else {
        return;
    };
    let color = state.color;
    let task_snapshot = task.clone();
    let bars_index = state.bars.iter().position(|bars| bars.id == id).unwrap_or_else(|| {
        let bar = state.multi.add(ProgressBar::new_spinner());
        state.bars.push(ProgressTaskBars {
            id: id.to_string(),
            bar,
            details: VecDeque::new(),
        });
        state.bars.len() - 1
    });

    let bar = &state.bars[bars_index].bar;
    match task_snapshot.status {
        ProgressTaskStatus::Running => {
            if bar.is_finished() {
                bar.reset();
            }
            bar.set_style(running_progress_style());
            bar.set_message(task_snapshot.message);
            bar.enable_steady_tick(Duration::from_millis(SPINNER_TICK_INTERVAL_MS));
        },
        ProgressTaskStatus::Succeeded | ProgressTaskStatus::Failed => {
            bar.finish_with_message(render_progress_task_line(&task_snapshot, color));
        },
    }
}

fn sync_progress_task_detail_bars(state: &mut ProgressTasksState, id: &str, max_details: usize) {
    let Some(task) = state.tasks.iter().find(|task| task.id == id) else {
        return;
    };
    let Some(bars_index) = state.bars.iter().position(|bars| bars.id == id) else {
        return;
    };

    while state.bars[bars_index].details.len() > max_details {
        if let Some(bar) = state.bars[bars_index].details.pop_front() {
            state.multi.remove(&bar);
        }
    }

    while state.bars[bars_index].details.len() < task.details.len() {
        let bar = state.multi.add(ProgressBar::new_spinner());
        bar.set_style(detail_progress_style());
        state.bars[bars_index].details.push_back(bar);
    }

    for (bar, detail) in state.bars[bars_index].details.iter().zip(task.details.iter()) {
        bar.set_message(render_progress_detail_line(detail, state.color));
    }
}

fn running_progress_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .tick_strings(SPINNER_TICKS)
        .template("{spinner:.cyan} {msg}")
        .expect("progress task spinner template should be valid")
}

fn detail_progress_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .template("{msg}")
        .expect("progress detail template should be valid")
}

pub fn create_spinner(message: &str) -> ProgressBar {
    create_spinner_inner(message, false)
}

pub fn create_spinner_with_elapsed(message: &str) -> ProgressBar {
    create_spinner_inner(message, true)
}

fn create_spinner_inner(message: &str, elapsed: bool) -> ProgressBar {
    let progress_bar = ProgressBar::new_spinner();
    let template = if elapsed {
        "{spinner:.cyan} {msg} {elapsed_precise}"
    } else {
        "{spinner:.cyan} {msg}"
    };
    progress_bar.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(SPINNER_TICKS)
            .template(template)
            .expect("spinner template should be valid"),
    );
    progress_bar.set_message(message.to_string());
    progress_bar.enable_steady_tick(Duration::from_millis(SPINNER_TICK_INTERVAL_MS));
    progress_bar
}

pub fn prompt_select(
    title: &str,
    options: &[SelectOption<'_>],
    default_index: usize,
    color: bool,
) -> io::Result<usize> {
    if options.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt_select requires at least one option",
        ));
    }

    let mut selected = first_enabled_index(options, default_index);
    run_menu(|stdout, rendered_lines| loop {
        redraw_menu(stdout, *rendered_lines, |stdout| {
            render_select_menu(stdout, title, options, selected, color)
        })?;
        *rendered_lines = menu_line_count(options);

        match read_menu_key()? {
            MenuKey::Up => selected = previous_enabled_index(options, selected),
            MenuKey::Down => selected = next_enabled_index(options, selected),
            MenuKey::Confirm => {
                if options[selected].disabled {
                    continue;
                }
                return Ok(selected);
            },
            MenuKey::Cancel => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "setup cancelled"));
            },
            MenuKey::Toggle | MenuKey::Other => {},
        }
    })
}

pub fn prompt_multi_select(
    title: &str,
    options: &[SelectOption<'_>],
    default_selected: &[bool],
    color: bool,
) -> io::Result<Vec<usize>> {
    if options.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt_multi_select requires at least one option",
        ));
    }

    let mut selected: Vec<bool> = options
        .iter()
        .enumerate()
        .map(|(idx, _)| default_selected.get(idx).copied().unwrap_or(false))
        .collect();
    if !selected.iter().any(|value| *value) {
        selected[0] = true;
    }

    let mut cursor_index = 0usize;
    run_menu(|stdout, rendered_lines| loop {
        redraw_menu(stdout, *rendered_lines, |stdout| {
            render_multi_select_menu(stdout, title, options, &selected, cursor_index, color)
        })?;
        *rendered_lines = menu_line_count(options);

        match read_menu_key()? {
            MenuKey::Up => cursor_index = cursor_index.saturating_sub(1),
            MenuKey::Down => {
                if cursor_index + 1 < options.len() {
                    cursor_index += 1;
                }
            },
            MenuKey::Toggle => {
                selected[cursor_index] = !selected[cursor_index];
                if !selected.iter().any(|value| *value) {
                    selected[cursor_index] = true;
                }
            },
            MenuKey::Confirm => {
                return Ok(selected
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, checked)| checked.then_some(idx))
                    .collect());
            },
            MenuKey::Cancel => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "setup cancelled"));
            },
            MenuKey::Other => {},
        }
    })
}

pub fn print_oidc_browser_prompt(provider: &str, authorization_url: &str, color: bool) {
    print_banner("OIDC browser login", Some("Authenticate with your identity provider."), color);
    println!("{} {}", prompt_label("Provider:", color), style_value(provider, color));
    println!("{}", style_info("Open this URL to continue:", color));
    println!("{}", terminal_hyperlink(authorization_url, color));
    println!();
}

pub fn print_oidc_device_prompt(
    provider: &str,
    verification_uri: &str,
    complete_uri: Option<&str>,
    user_code: &str,
    color: bool,
) {
    print_banner("OIDC device login", Some("Authenticate from any device."), color);
    println!("{} {}", prompt_label("Provider:", color), style_value(provider, color));
    println!("{}", style_info("Open this URL on any device:", color));
    println!("{}", terminal_hyperlink(complete_uri.unwrap_or(verification_uri), color));
    println!("{} {}", prompt_label("User code:", color), style_value(user_code, color));
    println!();
}

fn run_menu<T>(mut f: impl FnMut(&mut io::Stdout, &mut usize) -> io::Result<T>) -> io::Result<T> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    stdout.execute(cursor::Hide)?;

    let mut rendered_lines = 0usize;
    let result = f(&mut stdout, &mut rendered_lines);
    let clear_result = clear_rendered_menu(&mut stdout, rendered_lines);
    let _ = stdout.execute(cursor::Show);
    let _ = terminal::disable_raw_mode();

    match (result, clear_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn redraw_menu(
    stdout: &mut io::Stdout,
    rendered_lines: usize,
    render: impl FnOnce(&mut io::Stdout) -> io::Result<()>,
) -> io::Result<()> {
    clear_rendered_menu(stdout, rendered_lines)?;
    stdout.execute(cursor::MoveToColumn(0))?;
    render(stdout)?;
    stdout.flush()
}

fn clear_rendered_menu(stdout: &mut io::Stdout, rendered_lines: usize) -> io::Result<()> {
    for _ in 0..rendered_lines {
        stdout.execute(cursor::MoveUp(1))?;
        stdout.execute(cursor::MoveToColumn(0))?;
        stdout.execute(terminal::Clear(ClearType::CurrentLine))?;
    }
    stdout.flush()
}

fn render_select_menu(
    stdout: &mut io::Stdout,
    title: &str,
    options: &[SelectOption<'_>],
    selected: usize,
    color: bool,
) -> io::Result<()> {
    menu_line(stdout, style_accent(title, color))?;
    menu_blank_line(stdout)?;
    for (idx, option) in options.iter().enumerate() {
        let cursor = if idx == selected { ">" } else { " " };
        let radio = if idx == selected { "(*)" } else { "( )" };
        let label = if option.disabled {
            style_muted(option.label, color)
        } else if idx == selected {
            style_value(option.label, color)
        } else {
            option.label.to_string()
        };
        menu_line(stdout, format!("  {cursor} {radio} {label}"))?;
        if let Some(description) = option.description {
            menu_line(stdout, format!("        {}", style_muted(description, color)))?;
        }
    }
    menu_blank_line(stdout)?;
    menu_line(stdout, style_muted("  Up/Down move, Enter confirm, Esc cancel", color))
}

fn first_enabled_index(options: &[SelectOption<'_>], preferred: usize) -> usize {
    if options.is_empty() {
        return 0;
    }
    if !options[preferred.min(options.len() - 1)].disabled {
        return preferred.min(options.len() - 1);
    }
    next_enabled_index(options, options.len() - 1)
}

fn next_enabled_index(options: &[SelectOption<'_>], current: usize) -> usize {
    if options.is_empty() {
        return 0;
    }
    let len = options.len();
    for offset in 1..=len {
        let idx = (current + offset) % len;
        if !options[idx].disabled {
            return idx;
        }
    }
    current
}

fn previous_enabled_index(options: &[SelectOption<'_>], current: usize) -> usize {
    if options.is_empty() {
        return 0;
    }
    let len = options.len();
    for offset in 1..=len {
        let idx = (current + len - offset) % len;
        if !options[idx].disabled {
            return idx;
        }
    }
    current
}

fn render_multi_select_menu(
    stdout: &mut io::Stdout,
    title: &str,
    options: &[SelectOption<'_>],
    selected: &[bool],
    cursor_index: usize,
    color: bool,
) -> io::Result<()> {
    menu_line(stdout, style_accent(title, color))?;
    menu_blank_line(stdout)?;
    for (idx, option) in options.iter().enumerate() {
        let cursor = if idx == cursor_index { ">" } else { " " };
        let checkbox = if selected[idx] { "[x]" } else { "[ ]" };
        let label = if idx == cursor_index {
            style_value(option.label, color)
        } else {
            option.label.to_string()
        };
        menu_line(stdout, format!("  {cursor} {checkbox} {label}"))?;
        if let Some(description) = option.description {
            menu_line(stdout, format!("        {}", style_muted(description, color)))?;
        }
    }
    menu_blank_line(stdout)?;
    menu_line(
        stdout,
        style_muted("  Up/Down move, Space toggle, Enter confirm, Esc cancel", color),
    )
}

fn menu_line_count(options: &[SelectOption<'_>]) -> usize {
    let option_lines = options
        .iter()
        .map(|option| 1 + usize::from(option.description.is_some()))
        .sum::<usize>();
    // title + blank + options/descriptions + blank + controls
    option_lines + 4
}

fn menu_line(stdout: &mut io::Stdout, line: impl Display) -> io::Result<()> {
    // Raw mode does not guarantee that '\n' also returns the cursor to column 0.
    write!(stdout, "{line}\r\n")
}

fn menu_blank_line(stdout: &mut io::Stdout) -> io::Result<()> {
    write!(stdout, "\r\n")
}

enum MenuKey {
    Up,
    Down,
    Toggle,
    Confirm,
    Cancel,
    Other,
}

fn read_menu_key() -> io::Result<MenuKey> {
    loop {
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                return Ok(match key.code {
                    KeyCode::Up | KeyCode::Char('k') => MenuKey::Up,
                    KeyCode::Down | KeyCode::Char('j') => MenuKey::Down,
                    KeyCode::Char(' ') => MenuKey::Toggle,
                    KeyCode::Enter => MenuKey::Confirm,
                    KeyCode::Esc => MenuKey::Cancel,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        MenuKey::Cancel
                    },
                    _ => MenuKey::Other,
                });
            },
            _ => {},
        }
    }
}

fn terminal_hyperlink(url: &str, color: bool) -> String {
    if !color {
        return url.to_string();
    }
    format!("\x1b]8;;{url}\x1b\\{}\x1b]8;;\x1b\\", url.underline())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_progress_task_includes_indented_details() {
        let task = ProgressTask {
            id:      "server".into(),
            message: "Starting local KalamDB server...".into(),
            status:  ProgressTaskStatus::Running,
            details: vec!["listening on 127.0.0.1:2900".into()],
        };

        let lines = render_progress_task(&task, false);

        assert_eq!(lines[0], "⠋ Starting local KalamDB server...");
        assert_eq!(lines[1], "    listening on 127.0.0.1:2900");
    }

    #[test]
    fn progress_tasks_keep_latest_details() {
        let tasks = ProgressTasks::new_hidden(false);
        tasks.update_task("server", ProgressTaskStatus::Running, "Starting server");

        for line in ["one", "two", "three"] {
            tasks.push_task_detail("server", line, 2);
        }

        let lines = tasks.rendered_lines();
        assert_eq!(lines, vec!["⠋ Starting server", "    two", "    three"]);
    }

    #[test]
    fn progress_tasks_state_does_not_duplicate_tasks() {
        let tasks = ProgressTasks::new_hidden(false);

        tasks.update_task("environment", ProgressTaskStatus::Succeeded, "Environment ready");
        tasks.update_task("schema-source", ProgressTaskStatus::Succeeded, "Schema source found");
        tasks.push_task_detail("server", "ignored before task exists", 8);

        let lines = tasks.rendered_lines();
        assert_eq!(lines, vec!["✓ Environment ready", "✓ Schema source found"]);
    }

    #[test]
    fn alert_action_required_does_not_panic() {
        alert_action_required();
    }
}
